//! The character / stage / music select of the versus modes (`CharSel_Run(2)`, src/menu/char_select.c, progress
//! mode 39): two seven-column character reels with forms, item sets and costumes, then the stage reel and the
//! music list. It ends by writing the battle setup, the chosen item set's items taken from the save.
//!
//! An item set is looked at in the side's item panel (`ItemPanel_*`) and an item in the details page
//! (`ItemHelp_*`, `item_panel`). Nothing is saved yet, so the sets are those of a new save (empty) and the saved
//! custom characters are never listed.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::draw::ScreenQuad;
use crate::flash::{
    ClipRef, FLASH_PAD, Flash, PROP_MASK, PROP_MASK_SET, PROP_VISIBLE, Uv, screen_quads,
};
use crate::font::Font;
use crate::item_panel::{ItemAssets, ItemHelp, ItemPanel, help_textures, panel_textures};
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, image_indices, tex};
use crate::rng::GameRandom;
use crate::save::SaveData;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_RIGHT, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame,
    TitleSound,
};
use crate::windows::{IconWin, TextBox, TextDraw, draw_texts};
use bt3_data::chara_data::ChrTable;

/// `CharSel_Run(2)` unpacks section 2 of gMenuArc5.
pub const ARCHIVE_SECTION: usize = 2;
pub const FACE_FILE: u32 = 0x2F9;
pub const STAGE_FILE: u32 = 0x39D;
pub(crate) const BGM_FIRST: u32 = 0x10B16;
pub(crate) const BGM_RANDOM_FIRST: u32 = 0x10B1E;
pub(crate) const FADE_FRAMES: i32 = 0x14;

pub(crate) const COLS: usize = 7;
pub(crate) const STAGE_COLS: i32 = 6;
pub(crate) const STAGE_BGM_COL: i32 = 6;
const FORM_MAX: usize = 7;
/// `MCHR_*` / `CHRGRID_ID_*` marker cells.
pub(crate) const ID_RANDOM: i32 = 0xA1;
pub(crate) const ID_LOCKED: i32 = 0xA2;
pub(crate) const ID_CUSTOM: i32 = 0xA3;
pub(crate) const ID_EMPTY: i32 = 0xA4;
pub(crate) const CELL_MAX: usize = 165;
const CUSTOM_MAX: usize = 14;
pub(crate) const STAGE_RANDOM: i32 = 0x23;
pub(crate) const STAGE_LOCKED: i32 = 0x24;
pub(crate) const BGM_RANDOM: i32 = 0x18;
pub(crate) const BGM_LOCKED: i32 = 0x19;

const FLAG_STARTED: u32 = 2;
const FLAG_STAGE_PHASE: u32 = 4;
const FLAG_LEAVING: u32 = 8;
const FLAG_10: u32 = 0x10;
const FLAG_STAGE_CHANGE: u32 = 0x20;
const FLAG_STAGE_READY: u32 = 0x40;

const SIDE_FACE_CHANGE: u32 = 1;
const SIDE_FACE_READY: u32 = 2;
const SIDE_FORMS: u32 = 0x40;
const SIDE_CUSTOM: u32 = 0x80;
const SIDE_ITEMS: u32 = 0x800;

const ST_GRID: i32 = 0;
const ST_FORM: i32 = 1;
const ST_ITEMSET: i32 = 2;
const ST_ITEMS: i32 = 3;
const ST_ITEMINFO: i32 = 4;
const ST_COLOR: i32 = 5;
const ST_CUSTOM: i32 = 6;
const ST_DONE: i32 = 7;
const ST_STAGE: i32 = 8;
const ST_BGM: i32 = 9;

const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;
const LOAD_IDLE: i32 = 4;
const LOAD_ABORT: i32 = 5;
const LOAD_RESTART: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clip {
    Chip,
    FormChip,
    CustomPlate,
    ColorPlate,
    CustomChip,
    StageChip,
}

/// `gCharSel->tex[n]` of movie 0 (`None` for the slots filled at run time: chips, portraits).
pub(crate) fn flash_textures(players: i32) -> [ImageTexture; 49] {
    let mut t = [None; 49];
    t[0] = tex(47, 0);
    t[17] = tex(1, 0);
    t[16] = tex(1, 1);
    t[18] = tex(2, 1);
    t[20] = tex(3, 0);
    t[19] = tex(4, 0);
    t[9] = tex(5, 0);
    t[40] = tex(5, 1);
    if players == 1 {
        t[10] = tex(50, 0);
    }
    t[39] = tex(6, 0);
    t[38] = tex(7, 0);
    t[2] = tex(8, 1);
    t[4] = tex(8, 2);
    t[5] = tex(8, 4);
    t[6] = tex(8, 3);
    t[3] = tex(9, 1);
    t[1] = tex(9, 0);
    t[21] = tex(10, 0);
    t[24] = tex(11, 0);
    t[26] = tex(11, 1);
    t[25] = tex(11, 2);
    t[27] = tex(11, 3);
    t[47] = tex(12, 0);
    t[48] = tex(12, 1);
    t[7] = tex(21, 0);
    t
}

/// `gCharSel->sideTex[side]`, movies 1 and 2.
pub(crate) fn side_textures(side: usize) -> [ImageTexture; 25] {
    let mut t = [None; 25];
    t[0] = tex(17 + side, 0);
    t[2] = tex(17 + side, 1);
    t[1] = tex(19 + side, 1);
    t[4] = tex(19 + side, 2);
    t[5] = tex(19 + side, 4);
    t[6] = tex(19 + side, 3);
    t[7] = tex(21, 0);
    t[3] = tex(22, 0);
    t[10] = tex(23, 0);
    t[17] = tex(23, 1);
    t
}

/// `gCharSel->plateTex[side]`, movies 3 and 4.
pub(crate) fn plate_textures(side: usize) -> [ImageTexture; 11] {
    let mut t = [None; 11];
    t[4] = tex(23, 1);
    t[9] = tex(48 + side, 0);
    t[0] = tex(48 + side, 1);
    t[7] = tex(26 + side, 0);
    t[3] = tex(26 + side, 1);
    t[2] = tex(28, 0);
    t[5] = tex(28, 1);
    t[6] = tex(29, 0);
    t[8] = tex(29, 1);
    t[10] = tex(30 + side, 0);
    t[1] = tex(30 + side, 1);
    t
}

/// `gTeamSel->tex`: movie 0 as the character select has it, with the DP gauges' textures in a DP battle.
pub(crate) fn team_flash_textures(players: i32, dp: bool) -> [ImageTexture; 49] {
    let mut t = flash_textures(players);
    if dp {
        t[28] = tex(13, 0);
        t[31] = tex(13, 1);
        t[29] = tex(14, 0);
        t[30] = tex(14, 1);
    }
    t
}

/// `gTeamSel->sideTex[side]`, movies 3 and 4: the character select's reels, with the cost digits in a DP battle.
pub(crate) fn team_side_textures(side: usize, dp: bool) -> [ImageTexture; 25] {
    let mut t = side_textures(side);
    if dp {
        t[9] = tex(16, 2);
    }
    t
}

/// `gTeamSel->teamTex[side]`, movies 1 and 2 (the same for both sides; slots 1 and 4..7 are the members' chips).
pub(crate) fn team_textures(dp: bool) -> [ImageTexture; 13] {
    let mut t = [None; 13];
    t[0] = tex(34, 0);
    t[8] = tex(35, 0);
    t[11] = tex(35, 1);
    t[10] = tex(35, 2);
    t[9] = tex(36, 0);
    t[12] = tex(36, 1);
    t[3] = tex(23, 0);
    if dp {
        t[2] = tex(16, 2);
    }
    t
}

/// `IconWin_Init(MPACK_AT(res, 54), res 55)`.
pub(crate) const ICON_TEXTURES: [ImageTexture; 5] = [
    Some(((54, 1), 1)),
    Some(((54, 1), 0)),
    tex(55, 0),
    Some(((54, 1), 2)),
    tex(55, 1),
];

/// A cell of the character grid (`MChrCell`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) id: i32,
    pub(crate) form_count: i32,
    pub(crate) form: [i32; FORM_MAX],
}

/// What `CharSel_Init` reads from its unpacked section.
pub struct CharSelAssets {
    pub data: CharSelData,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
}

#[derive(Clone, Debug)]
pub struct CharSelData {
    /// Sections 15 (background), 24 / 25 (reels), 32 / 33 (plates), 54 part 2 (icon window).
    pub fods: [Vec<u8>; 5],
    pub icon_fod: Vec<u8>,
    /// Section 43 / 44: character and form names, one line per character id.
    pub name_text: Vec<u8>,
    pub form_text: Vec<u8>,
    /// Sections 37 / 38: the team select's member lists (`TeamSel`'s movies 1 and 2).
    pub team_fods: [Vec<u8>; 2],
    /// Section 42: the master character grid.
    pub(crate) master: Vec<Cell>,
    /// Section 39: the stage grid; section 56: the music list.
    pub(crate) stage_ids: Vec<i32>,
    pub(crate) bgm_ids: Vec<i32>,
    /// Sections 53 and 51: the item panels and the details page.
    pub items: ItemAssets,
    pub layout: CharSelLayout,
}

/// Texture indices (the clamped texture, followed by its wrapping twin).
#[derive(Clone, Debug, Default)]
pub struct CharSelLayout {
    pub(crate) flash: [Vec<Option<usize>>; 2],
    pub(crate) side: [Vec<Option<usize>>; 2],
    pub(crate) plate: [Vec<Option<usize>>; 2],
    pub(crate) icon: Vec<Option<usize>>,
    pub(crate) panel: [Vec<Option<usize>>; 2],
    pub(crate) help: Vec<Option<usize>>,
    /// Entry 0 of section 55 (the icon window's icons).
    pub(crate) icons: usize,
    /// Per character id + 1: the reel chip (section 45); per stage id + 1: the stage chip (section 46).
    pub(crate) chips: Vec<usize>,
    pub(crate) stage_chips: Vec<usize>,
    /// The team select's: movie 0 by [two pads][DP battle], the reels by [side][DP battle], the member lists by
    /// [side][DP battle].
    pub(crate) team_flash: [[Vec<Option<usize>>; 2]; 2],
    pub(crate) team_side: [[Vec<Option<usize>>; 2]; 2],
    pub(crate) team: [[Vec<Option<usize>>; 2]; 2],
}

impl CharSelLayout {
    pub fn new(first: impl Fn(FileKey) -> usize) -> Self {
        Self {
            flash: [0, 1].map(|players| image_indices(&flash_textures(players), &first)),
            side: [0, 1].map(|s| image_indices(&side_textures(s), &first)),
            plate: [0, 1].map(|s| image_indices(&plate_textures(s), &first)),
            icon: image_indices(&ICON_TEXTURES, &first),
            panel: [0, 1].map(|s| image_indices(&panel_textures(s), &first)),
            help: image_indices(&help_textures(), &first),
            icons: first((55, 0)),
            chips: (1..=CELL_MAX).map(|n| first((45, n))).collect(),
            stage_chips: (1..=38).map(|n| first((46, n))).collect(),
            team_flash: [0, 1].map(|players| {
                [false, true].map(|dp| image_indices(&team_flash_textures(players, dp), &first))
            }),
            team_side: [0, 1]
                .map(|s| [false, true].map(|dp| image_indices(&team_side_textures(s, dp), &first))),
            team: [0, 1].map(|_| [false, true].map(|dp| image_indices(&team_textures(dp), &first))),
        }
    }
}

/// `MPACK_WORD` (the entry count) and `per` words per entry from +0x10.
fn table(data: &[u8], per: usize) -> Result<Vec<i32>, Box<dyn Error>> {
    let word = |i: usize| -> Result<i32, Box<dyn Error>> {
        let b = data.get(i * 4..i * 4 + 4).ok_or("table is cut short")?;
        Ok(i32::from_le_bytes(b.try_into()?))
    };
    let count = word(0)?.max(0) as usize;
    (0..count * per).map(|i| word(4 + i)).collect()
}

impl CharSelAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let mut tables: Vec<Vec<ImageTexture>> = vec![
            flash_textures(1).to_vec(),
            side_textures(0).to_vec(),
            side_textures(1).to_vec(),
            plate_textures(0).to_vec(),
            plate_textures(1).to_vec(),
            ICON_TEXTURES.to_vec(),
            panel_textures(0).to_vec(),
            panel_textures(1).to_vec(),
            help_textures().to_vec(),
            team_flash_textures(1, true).to_vec(),
            team_side_textures(0, true).to_vec(),
            team_side_textures(1, true).to_vec(),
            team_textures(true).to_vec(),
        ];
        tables.push((1..=CELL_MAX).map(|n| Some(((45, n), 0))).collect());
        tables.push((1..=38).map(|n| Some(((46, n), 0))).collect());
        let files = decode_files(res, tables.iter().map(Vec::as_slice))?;
        let words = table(pack_section(res, 42)?, 9)?;
        let master = words
            .chunks_exact(9)
            .map(|w| Cell {
                id: w[0],
                form_count: w[1],
                form: w[2..9].try_into().unwrap(),
            })
            .collect();
        let stage_ids = table(pack_section(res, 39)?, 1)?;
        let bgm_ids = table(pack_section(res, 56)?, 1)?;
        let fod =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        Ok(Self {
            data: CharSelData {
                fods: [fod(15)?, fod(24)?, fod(25)?, fod(32)?, fod(33)?],
                team_fods: [fod(37)?, fod(38)?],
                icon_fod: pack_section(pack_section(res, 54)?, 2)?.to_vec(),
                name_text: fod(43)?,
                form_text: fod(44)?,
                master,
                stage_ids,
                bgm_ids,
                items: ItemAssets::parse(res)?,
                layout: CharSelLayout::default(),
            },
            files,
        })
    }
}

/// The first 0x30 bytes of a side's choice, kept in `gProgress` between visits (`CharSelPick`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    pub col: i32,
    pub row: i32,
    pub form: i32,
    pub custom_col: i32,
    pub custom_row: i32,
    /// Item set: 0 none, 1..3 a set of `gSaveData->custom[]`.
    pub custom_plate: i32,
    /// Costume.
    pub color_plate: i32,
    /// The character finally chosen.
    pub picked: i32,
    pub items: [u16; 8],
}

/// The character select's fields of `gProgress`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharSelProgress {
    /// Each side's choice; the team select's first member.
    pub side: [Pick; 2],
    /// The team select's other four members of each side (`gProgress->team[side].member[1..]`, the character
    /// select's pick being member 0).
    pub team: [[Pick; 4]; 2],
    /// Index in the stage grid.
    pub stage: i32,
    /// Id in the music list.
    pub bgm: i32,
}

/// One side of the battle as `BattleSetup_SetSide` / `BattleSetup_SetMember` get it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SideSetup {
    /// 0 a pad, 2 the computer.
    pub control: i32,
    pub pad: i32,
    pub change_allowed: i32,
    pub chara: i32,
    pub costume: i32,
    pub cpu_level: i32,
    pub items: [u16; 8],
    /// The team members' item sets (1-based ids).
    pub team_items: [[u16; 8]; 4],
    /// The team after the lead (`BattleSide.members[1..]`): character and costume, `team_count` of them.
    pub team: [(i32, i32); 4],
    pub team_count: i32,
    /// `BattleSide.switchEnabled`: the switch gauge fills.
    pub switch_enabled: i32,
    /// Per member (the lead first) as the story scripts set them: its own CPU level, the health it starts with
    /// and its damaged variant; `None`: every member at `cpu_level`, full health, variant 0.
    pub members: Option<[MemberStart; 5]>,
}

/// What `BtlMember_Init` keeps of a member beyond its character, costume and items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberStart {
    pub cpu_level: i32,
    /// Percent of the health maximum.
    pub health: f32,
    pub variant: i32,
}

/// One opponent of mode 3's pool (`BattleSetup_SetPoolMember`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PoolMember {
    pub chara: i32,
    pub costume: i32,
    pub cpu_level: i32,
    /// 1-based item ids.
    pub items: [u16; 8],
}

/// What the menus hand to the battle (`BattleSetup_SetRule`, the two sides and mode 3's opponent pool).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BattleSetup {
    pub screen_mode: i32,
    pub mode: i32,
    pub bgm: i32,
    pub time_limit: i32,
    pub announcer: i32,
    pub stage: i32,
    pub stage_change: i32,
    pub sides: [SideSetup; 2],
    /// `Battle_GetWork5A8()`: the opponents side 1 fields one after the other in mode 3.
    pub pool: Vec<PoolMember>,
    /// `gProgress->tutorial`: the tutorial whose controls mode 5's pause menu shows.
    pub tutorial: i32,
    /// `BattleSetup_FinishEx(1)`: the bonuses of a side at a pad clamped to -20..60 (Sim Dragon's fights).
    pub wide: bool,
}

impl BattleSetup {
    /// `BattleSetup_FinishEx`'s mode rules: `BattleSetup_FixForMode` (split screen only in modes 0 and 4, mode 9
    /// without a time limit and mode 7 with limit 5; one fighter a side in modes 4 and 6, and for the player in mode
    /// 3; the player at a pad and the computer on side 1 in modes 3 and 6, side 1 a dummy at CPU level -1 in mode 6;
    /// the computer on both sides in modes 7 and 9, at level 29 in 9; mode 9 is then played as 6), then mode 3's
    /// first opponent as side 1's fighter. No menu sets up mode 9, and its AI type 0 is not kept.
    pub fn finish(&mut self) {
        let mode = self.mode;
        if mode != 0 && mode != 4 && self.screen_mode == 1 {
            self.screen_mode = 0;
        }
        if mode == 9 {
            self.time_limit = 0;
        }
        if mode == 7 {
            self.time_limit = 5;
        }
        for (i, side) in self.sides.iter_mut().enumerate() {
            match mode {
                9 => {
                    if side.control != 2 {
                        side.control = 2;
                        side.cpu_level = 29;
                    }
                }
                6 => {
                    side.team_count = 0;
                    if i == 0 {
                        side.control = 0;
                    } else {
                        side.control = 2;
                        side.cpu_level = -1;
                    }
                }
                4 => side.team_count = 0,
                3 => {
                    if i == 0 {
                        side.team_count = 0;
                        side.control = 0;
                    } else {
                        side.control = 2;
                    }
                }
                7 => side.control = 2,
                _ => {}
            }
        }
        if mode == 9 {
            self.mode = 6;
        }
        if self.mode == 3
            && let Some(first) = self.pool.first()
        {
            let side = &mut self.sides[1];
            side.chara = first.chara;
            side.costume = first.costume;
            side.cpu_level = first.cpu_level;
            side.items = first.items;
        }
    }

    /// `SurvSel_SetupBattle` for course `course`, then `Ub_SetupSolo` with the player's fighter: mode 3 against
    /// one computer fighter backed by the course's fifty opponents. A random announcer, stage or opponent is drawn
    /// with `Rand_Range` in that order; the random music (0x18) with `rand() % 24` (`BattleSetup_SetRule`).
    pub fn survival(
        data: &bt3_data::survival::SurvivalData,
        course: usize,
        player: (i32, i32, [u16; 8]),
        random: &mut GameRandom,
        libc: &mut crate::rng::LibcRandom,
    ) -> Option<Self> {
        use bt3_data::survival::RANDOM;
        let def = data.courses.get(course)?;
        let announcer = if def.announcer == RANDOM {
            random.range(8) as i32
        } else {
            def.announcer
        };
        let stage = if def.stage == RANDOM {
            random.range(0x23) as i32
        } else {
            def.stage
        };
        let bgm = if def.bgm == RANDOM { 0x18 } else { def.bgm };
        let bgm = if bgm == 24 { libc.rand() % 24 } else { bgm };
        let mut pool = Vec::with_capacity(def.opp.len());
        for &i in &def.opp {
            let opp = data.opponents.get(usize::try_from(i).ok()?)?;
            let chara = if opp.chara == RANDOM {
                let n = random.range(data.random_charas.len() as u32) as usize;
                *data.random_charas.get(n)?
            } else {
                opp.chara
            };
            pool.push(PoolMember {
                chara,
                costume: opp.costume,
                cpu_level: opp.cpu_level,
                items: opp.items(),
            });
        }
        let side = |control, pad, change_allowed| SideSetup {
            control,
            pad,
            change_allowed,
            switch_enabled: 1,
            ..SideSetup::default()
        };
        let mut setup = Self {
            wide: false,
            screen_mode: 0,
            mode: 3,
            bgm,
            time_limit: def.time_limit,
            announcer,
            stage,
            stage_change: i32::from(def.stage_change != 0),
            sides: [
                side(0, 0, 1),
                side(2, 1, i32::from(def.change_allowed != 0)),
            ],
            pool,
            tutorial: 0,
        };
        let (chara, costume, items) = player;
        setup.sides[0].chara = chara;
        setup.sides[0].costume = costume;
        setup.sides[0].items = items;
        setup.finish();
        Some(setup)
    }
}

/// `gProgress->demoCount` (attract battles since the opening movie) and `demoPick` (the last pairing, -1 at
/// start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoProgress {
    pub count: u32,
    pub pick: i32,
}

impl Default for DemoProgress {
    fn default() -> Self {
        Self { count: 0, pick: -1 }
    }
}

/// `Demo_SetupBattle`'s pairings: the two characters and the stage.
const DEMO_PAIRS: [(i32, i32, i32); 9] = [
    (0xA, 0x93, 0x1D),
    (0x2A, 0x6C, 6),
    (0x19, 0x5C, 2),
    (0x27, 0x60, 1),
    (0x17, 0x6F, 1),
    (0x49, 0x5B, 0x10),
    (0, 0x1E, 0x19),
    (0x32, 0x72, 5),
    (5, 0x25, 1),
];

impl BattleSetup {
    /// `Demo_SetupBattle`: the attract battle (mode 7), two computer fighters at CPU level 13 in one of nine fixed
    /// pairings on its stage, never the last one again (`Rand_Range(9)` until it differs), with music 8..16
    /// (`Rand_Range(9)` once more), announcer 4 and time limit 5.
    pub fn demo(random: &mut GameRandom, last: &mut i32) -> Self {
        let pick = loop {
            let pick = random.range(9) as i32;
            if pick != *last {
                break pick;
            }
        };
        *last = pick;
        let (a, b, stage) = DEMO_PAIRS[pick as usize];
        let bgm = random.range(9) as i32 + 8;
        let side = |pad, chara| SideSetup {
            control: 2,
            pad,
            change_allowed: 1,
            chara,
            cpu_level: crate::btl_menu::cpu_level_from_setting(2),
            switch_enabled: 1,
            ..SideSetup::default()
        };
        let mut setup = Self {
            mode: 7,
            bgm,
            time_limit: 5,
            announcer: 4,
            stage,
            sides: [side(0, a), side(1, b)],
            ..Self::default()
        };
        setup.finish();
        setup
    }
}

/// How `CharSel_Run` ended.
#[derive(Clone, Debug, PartialEq)]
pub enum CharSelResult {
    /// 0: back to the duel menu.
    Back,
    Battle(BattleSetup),
}

/// Loads the pictures the screen streams in: a portrait (file `FACE_FILE + id`, one texture, as a clamped /
/// wrapping pair) and a stage picture (file `STAGE_FILE + id`, eight tiles).
pub trait PictureLoader {
    fn portrait(&mut self, file_id: u32) -> Option<usize>;
    fn picture(&mut self, file_id: u32) -> Option<usize>;
    /// Every texture of a file as clamped / wrapping pairs: the first one's index.
    fn texture_file(&mut self, _file_id: u32) -> Option<usize> {
        None
    }
}

/// `CharSelSide`.
#[derive(Clone, Copy, Debug, Default)]
struct Side {
    pick: Pick,
    chip: [i32; 7],
    prev_chip: [i32; 7],
    flags: u32,
    chara: i32,
    state: i32,
    mask: i32,
}

/// `CharSelStage`.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    col: i32,
    row: i32,
    chip: [[i32; 6]; 2],
    stage: i32,
    state: i32,
    mask: i32,
    bgm: i32,
    bgm_playing: i32,
}

pub struct CharSelScreen {
    layout: CharSelLayout,
    name_text: Vec<u8>,
    form_text: Vec<u8>,
    flash: [Flash; 5],
    tex: Vec<Option<usize>>,
    side_tex: [Vec<Option<usize>>; 2],
    plate_tex: [Vec<Option<usize>>; 2],
    icon: IconWin,
    fade: ColorFade,
    pads: [PadRepeat; 2],
    flags: u32,
    face_state: i32,
    stage_state: i32,
    face_side: usize,
    timer: i32,
    side: [Side; 2],
    stage: Stage,
    master_count: [i32; 2],
    cells: [Vec<Cell>; 2],
    custom_count: [i32; 2],
    custom: [Vec<Cell>; 2],
    rows: [i32; 2],
    custom_rows: [i32; 2],
    color_count: [i32; 2],
    stage_rows: i32,
    players: i32,
    bg: [Option<usize>; 2],
    bg_alpha: [i32; 2],
    face_alpha: [f32; 2],
    turn: usize,
    stage_ids: Vec<i32>,
    bgm_ids: Vec<i32>,
    stage_count: i32,
    bgm_count: i32,
    face_mask: i32,
    /// The portrait / stage picture the loader is bringing in.
    loading_face: Option<usize>,
    loading_stage: Option<usize>,
    /// Common file 4's tables (`ChrTbl_WrapCostume`, `ChrTbl_GetItemSlots`, the items).
    chr: ChrTable,
    /// The item panels of both sides, the details page and the item it shows (`itemInfo`).
    panels: [ItemPanel; 2],
    help: ItemHelp,
    item_info: usize,
    progress: CharSelProgress,
    running: bool,
    /// `gProgress->mode == 0x2D`: the character select of free training (from the training menu's archive), one
    /// player, which sets up free training's battle.
    training: bool,
    done: Option<CharSelResult>,
}

impl CharSelScreen {
    /// `CharSel_Init(2)` and the first `ColorFade_StartIn`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &CharSelData,
        players: i32,
        progress: CharSelProgress,
        save: &SaveData,
        chr: ChrTable,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut flash = Vec::new();
        for fod in &data.fods {
            let mut f = Flash::create(fod)?;
            f.play(1);
            flash.push(f);
        }
        let flash: [Flash; 5] = flash
            .try_into()
            .map_err(|_| "the character select has five movies")?;
        let layout = data.layout.clone();
        let mut stage_ids = data.stage_ids.clone();
        stage_grid_apply_unlocks(&mut stage_ids, save);
        let mut bgm_ids = data.bgm_ids.clone();
        bgm_list_apply_unlocks(&mut bgm_ids, save);
        let mut bgm_count = bgm_ids.len() as i32 - 4;
        bgm_ids[bgm_count as usize - 1] = BGM_RANDOM;
        bgm_ids.truncate(bgm_count.max(0) as usize);
        bgm_count = bgm_ids.len() as i32;
        let (grid, custom, custom_count) = chr_grid_build(&data.master, save);
        let master_count = grid.len() as i32;
        let rows = (master_count + COLS as i32 - 1) / COLS as i32;
        let mut screen = Self {
            tex: layout.flash[usize::from(players == 1)].clone(),
            side_tex: layout.side.clone(),
            plate_tex: layout.plate.clone(),
            icon: IconWin::new(&data.icon_fod, layout.icon.clone())?,
            layout,
            name_text: data.name_text.clone(),
            form_text: data.form_text.clone(),
            flash,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pads: [PadRepeat::default(); 2],
            flags: 0,
            face_state: LOAD_IDLE,
            stage_state: LOAD_IDLE,
            face_side: 0,
            timer: 0,
            side: [Side::default(); 2],
            stage: Stage::default(),
            master_count: [master_count; 2],
            cells: [grid.clone(), grid],
            custom_count: [custom_count; 2],
            custom: [custom.clone(), custom],
            rows: [rows; 2],
            custom_rows: [0; 2],
            color_count: [0; 2],
            stage_rows: 6,
            players,
            bg: [None; 2],
            bg_alpha: [0x80, 0],
            face_alpha: [0.0; 2],
            turn: 0,
            stage_count: stage_ids.len() as i32,
            stage_ids,
            bgm_ids,
            bgm_count,
            face_mask: 0,
            loading_face: None,
            loading_stage: None,
            panels: [
                ItemPanel::new(0, &data.items, data.layout.panel[0].clone())?,
                ItemPanel::new(1, &data.items, data.layout.panel[1].clone())?,
            ],
            help: ItemHelp::new(&data.items, data.layout.help.clone())?,
            item_info: 0,
            chr,
            progress,
            running: true,
            training: false,
            done: None,
        };
        let s = &mut screen;
        s.side[0].pick = progress.side[0];
        s.side[1].pick = progress.side[1];
        s.stage.col = progress.stage % STAGE_COLS;
        s.stage.row = progress.stage / 6;
        for (i, &id) in s.bgm_ids.iter().enumerate() {
            if id == progress.bgm {
                s.stage.bgm = i as i32;
            }
        }
        for i in 0..2 {
            if !chr_grid_is_selectable(&s.cells[i], s.cell_index(i)) {
                s.side[i].pick = Pick {
                    col: i as i32,
                    ..Pick::default()
                };
                s.progress.side[i] = s.side[i].pick;
            }
        }
        if s.stage_ids
            .get(progress.stage as usize)
            .is_none_or(|&id| id >= STAGE_LOCKED)
        {
            s.progress.stage = 0;
            s.stage.col = 0;
            s.stage.row = 0;
        }
        s.set_row_chips(0);
        s.set_row_chips(1);
        s.set_stage_chips();
        for i in 0..2 {
            s.side[i].chara = s.side[i].chip[s.side[i].pick.col as usize];
        }
        s.stage.stage = s.stage_ids[(s.stage.col + s.stage.row * 6) as usize];
        s.stage.bgm_playing = s.stage.bgm;
        for i in 0..2 {
            s.tex[if i != 0 { 22 } else { 23 }] =
                loader.portrait(FACE_FILE + s.side[i].chara as u32);
            s.side[i].flags |= SIDE_FACE_READY;
        }
        let picture = loader.picture(STAGE_FILE + s.stage.stage as u32);
        s.bg = [picture; 2];
        s.flags |= FLAG_STAGE_READY;
        let id = s.bgm_ids[s.stage.bgm_playing as usize];
        let file = if id == BGM_RANDOM {
            BGM_RANDOM_FIRST + random.range(9)
        } else {
            BGM_FIRST + id as u32
        };
        sounds.push(TitleSound::Bgm(file));
        Ok(screen)
    }

    pub fn result(&self) -> Option<CharSelResult> {
        self.done.clone()
    }

    pub fn progress(&self) -> CharSelProgress {
        self.progress
    }

    fn cell_index(&self, side: usize) -> usize {
        let p = &self.side[side].pick;
        (p.col + p.row * COLS as i32) as usize
    }

    fn cell(&self, side: usize) -> Cell {
        self.cells[side][self.cell_index(side)]
    }

    fn custom_index(&self, side: usize) -> usize {
        let p = &self.side[side].pick;
        (p.custom_col + p.custom_row * COLS as i32) as usize
    }

    /// The reel chips: slots 8 and 11..16 of the side's movie, the old textures moved to 18..24.
    fn set_chips(&mut self, side: usize, ids: [i32; 7]) {
        const SLOT: [usize; 7] = [8, 11, 12, 13, 14, 15, 16];
        for (i, &id) in ids.iter().enumerate() {
            let s = &mut self.side[side];
            s.prev_chip[i] = s.chip[i];
            s.chip[i] = id;
            let chip = self.layout.chips.get(id as usize).copied();
            let tex = &mut self.side_tex[side];
            tex[18 + i] = tex[SLOT[i]];
            tex[SLOT[i]] = chip;
        }
    }

    /// `CharSel_SetRowChips`.
    fn set_row_chips(&mut self, side: usize) {
        let row = self.side[side].pick.row as usize;
        let ids = std::array::from_fn(|i| self.cells[side][i + row * COLS].id);
        self.set_chips(side, ids);
    }

    /// `CharSel_SetCellFormChips`.
    fn set_cell_form_chips(&mut self, side: usize) {
        let cell = self.cell(side);
        let ids = std::array::from_fn(|i| {
            if (i as i32) < cell.form_count {
                cell.form[i]
            } else {
                ID_EMPTY
            }
        });
        self.set_chips(side, ids);
    }

    /// `CharSel_SetCustomChips`.
    fn set_custom_chips(&mut self, side: usize) {
        let row = self.side[side].pick.custom_row as usize;
        let ids = std::array::from_fn(|i| self.custom[side][i + row * COLS].id);
        self.set_chips(side, ids);
    }

    /// `CharSel_SetFormChips`.
    fn set_form_chips(&mut self, side: usize) {
        let cell = self.custom[side][self.custom_index(side)];
        let ids = std::array::from_fn(|i| {
            if (i as i32) < cell.form_count {
                cell.form[i]
            } else {
                ID_EMPTY
            }
        });
        self.set_chips(side, ids);
    }

    /// `CharSel_SetStageChips`: slots 8 and 11..15 of movie 0, the old textures moved to 41..46.
    fn set_stage_chips(&mut self) {
        const SLOT: [usize; 6] = [8, 11, 12, 13, 14, 15];
        for (i, &slot) in SLOT.iter().enumerate() {
            let id = self.stage_ids[i + self.stage.row as usize * 6];
            self.stage.chip[1][i] = self.stage.chip[0][i];
            self.stage.chip[0][i] = id;
            self.tex[41 + i] = self.tex[slot];
            self.tex[slot] = self.layout.stage_chips.get(id as usize).copied();
        }
    }

    /// `CharSel_UpdateFaceLoad`: one step of the portrait loader.
    fn update_face_load(&mut self, loader: &mut dyn PictureLoader) {
        let fs = self.face_side;
        match self.face_state {
            LOAD_ABORT => {
                if self.side[fs ^ 1].flags & SIDE_FACE_CHANGE != 0 {
                    self.face_side ^= 1;
                }
                self.face_state = LOAD_RESTART;
            }
            LOAD_RESTART => {
                self.side[fs].flags &= !SIDE_FACE_CHANGE;
                self.face_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                self.loading_face = loader.portrait(FACE_FILE + self.side[fs].chara as u32);
                self.face_state = LOAD_READ;
            }
            LOAD_READ => self.face_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[if fs == 0 { 23 } else { 22 }] = self.loading_face;
                self.side[fs].flags |= SIDE_FACE_READY;
                if self.side[fs ^ 1].flags & SIDE_FACE_CHANGE != 0 {
                    self.face_side ^= 1;
                    self.face_state = LOAD_RESTART;
                } else {
                    self.face_state = LOAD_IDLE;
                }
            }
            LOAD_IDLE => {
                if self.stage_state != LOAD_IDLE {
                    return;
                }
                if self.side[0].flags & SIDE_FACE_CHANGE != 0 {
                    self.tex[23] = None;
                    self.face_side = 0;
                    self.face_state = LOAD_RESTART;
                } else if self.side[1].flags & SIDE_FACE_CHANGE != 0 {
                    self.tex[22] = None;
                    self.face_side = 1;
                    self.face_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `CharSel_RequestFace`.
    fn request_face(&mut self, side: usize) {
        if self.side[side].flags & SIDE_FACE_READY != 0 {
            self.side[side].flags ^= SIDE_FACE_READY;
        } else if self.face_side == side {
            self.face_state = LOAD_ABORT;
        }
        self.side[side].flags |= SIDE_FACE_CHANGE;
    }

    /// `CharSel_UpdateStageLoad`: one step of the stage picture loader.
    fn update_stage_load(&mut self, loader: &mut dyn PictureLoader) {
        match self.stage_state {
            LOAD_ABORT => self.stage_state = LOAD_RESTART,
            LOAD_RESTART => {
                self.flags &= !FLAG_STAGE_CHANGE;
                self.stage_state = LOAD_REQUEST;
            }
            LOAD_REQUEST => {
                self.loading_stage = loader.picture(STAGE_FILE + self.stage.stage as u32);
                self.stage_state = LOAD_READ;
            }
            LOAD_READ => self.stage_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.bg[0] = self.loading_stage;
                self.flags |= FLAG_STAGE_READY;
                self.stage_state = LOAD_IDLE;
            }
            LOAD_IDLE => {
                if self.face_state == LOAD_IDLE && self.flags & FLAG_STAGE_CHANGE != 0 {
                    self.stage_state = LOAD_RESTART;
                }
            }
            _ => {}
        }
    }

    /// `CharSel_RequestStage`: the old picture fades out over the new one once it is in.
    fn request_stage(&mut self) {
        if self.flags & FLAG_STAGE_READY != 0 {
            self.flags ^= FLAG_STAGE_READY;
        } else {
            self.stage_state = LOAD_ABORT;
        }
        self.flags |= FLAG_STAGE_CHANGE;
        self.bg[1] = self.bg[0];
    }

    /// `CharSel_ClipGoto(flash, side, kind, label)`.
    fn clip_goto(&mut self, flash: usize, side: usize, kind: Clip, label: &str) {
        let p = &self.side[side].pick;
        let name = match kind {
            Clip::Chip => format!("mc_chara_chip_{:03}", p.col),
            Clip::FormChip => format!("mc_chara_chip_{:03}", p.form),
            Clip::CustomPlate => format!("mc_custom_plate_{}", p.custom_plate + 1),
            Clip::ColorPlate => format!("mc_color_plate_{}", p.color_plate + 1),
            Clip::CustomChip => format!("mc_chara_chip_{:03}", p.custom_col),
            Clip::StageChip if self.stage.col == STAGE_BGM_COL => "mc_bgm_now".to_owned(),
            Clip::StageChip => format!("mc_map_chip_{:02}", self.stage.col),
        };
        let f = &mut self.flash[flash];
        let clip = f.find(None, &name);
        f.clip_goto_label(clip, label);
    }

    fn find(&self, flash: usize, parent: Option<&str>, name: &str) -> ClipRef {
        self.flash[flash].find(parent, name)
    }

    fn set_uv(&mut self, flash: usize, parent: Option<&str>, name: &str, uv: Uv) {
        let clip = self.find(flash, parent, name);
        self.flash[flash].clip_set_uv(clip, uv);
    }

    fn set_flags(&mut self, flash: usize, parent: Option<&str>, name: &str, props: u32, on: bool) {
        let clip = self.find(flash, parent, name);
        self.flash[flash].clip_set_flags(clip, props, on);
    }

    /// One frame of the `CharSel_Run` loop. `pads` are the pressed / held game buttons of pads 0 and 1.
    pub fn run_frame(
        &mut self,
        pads: [(u32, u32); 2],
        loader: &mut dyn PictureLoader,
        save: &SaveData,
        random: &mut GameRandom,
        font: &mut Font,
    ) -> TitleFrame {
        let mut frame = TitleFrame {
            quads: Vec::new(),
            sounds: Vec::new(),
        };
        if self.done.is_some() {
            return frame;
        }
        self.update_face_load(loader);
        self.update_stage_load(loader);
        let pad: [(u32, u32); 2] = std::array::from_fn(|i| {
            let (pressed, held) = pads[i];
            (pressed, self.pads[i].update(held, pressed))
        });
        self.fade.update();
        self.update();
        self.draw(font, &mut frame.quads);
        font.flush(&mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::BgmFadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            if self.face_state == LOAD_IDLE || self.stage_state == LOAD_IDLE {
                self.done = Some(if self.running {
                    CharSelResult::Battle(self.battle_setup(save, random))
                } else {
                    CharSelResult::Back
                });
            }
            return frame;
        }
        if self.flags & FLAG_10 != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.progress = CharSelProgress {
                    side: [self.side[0].pick, self.side[1].pick],
                    team: self.progress.team,
                    stage: self.stage.col + self.stage.row * STAGE_COLS,
                    bgm: self.bgm_ids[self.stage.bgm_playing as usize],
                };
            }
        } else {
            self.input(pad, save, random, &mut frame.sounds);
        }
        frame
    }

    /// `CharSel_Run` in progress mode 0x2D (`TrainMode_Main`): the screen is built from the training archive's
    /// section (`players` 0) and sets up free training.
    pub fn for_training(mut self) -> Self {
        self.training = true;
        self
    }

    /// The end of `CharSel_Run`: a random stage or music is drawn, then the battle setup; in mode 0x2D free
    /// training (mode 6: announcer 1, no time limit, no stage change, CPU level -1, changes allowed).
    fn battle_setup(&mut self, save: &SaveData, random: &mut GameRandom) -> BattleSetup {
        if self.stage.stage == STAGE_RANDOM {
            loop {
                self.stage.stage =
                    self.stage_ids[random.range(self.stage_count as u32 - 1) as usize];
                if self.stage.stage < STAGE_RANDOM {
                    break;
                }
            }
        }
        let mut bgm = self.bgm_ids[self.stage.bgm_playing as usize];
        if bgm == BGM_RANDOM {
            bgm = random.range(9) as i32 + 8;
        }
        if self.training {
            let side = |control, pad, i: usize| SideSetup {
                control,
                pad,
                change_allowed: 1,
                chara: self.side[i].pick.picked,
                costume: self.side[i].pick.color_plate,
                cpu_level: -1,
                items: self.side[i].pick.items,
                switch_enabled: 1,
                ..SideSetup::default()
            };
            // CSPROG->players is 0 in this mode: the pad against the computer.
            return BattleSetup {
                wide: false,
                screen_mode: 0,
                mode: 6,
                bgm,
                time_limit: 0,
                announcer: 1,
                stage: self.stage.stage,
                stage_change: 0,
                sides: [side(0, 0, 0), side(2, 1, 1)],
                pool: Vec::new(),
                tutorial: 0,
            };
        }
        let time_limit = match save.rules[0] {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 4,
            4 => 0,
            _ => 5,
        };
        let cpu_level = match save.rules[1] {
            0 => 0,
            1 => 6,
            2 => 13,
            3 => 21,
            4 => 29,
            _ => 0,
        };
        let flag = [save.rules[3] ^ 1, save.rules[4] ^ 1];
        let (control, pad, change) = match self.players {
            0 => ([0, 2], [0, 1], [1, flag[1]]),
            1 => ([0, 0], [0, 1], [1, 1]),
            _ => ([2, 2], [0, 1], flag),
        };
        BattleSetup {
            wide: false,
            screen_mode: i32::from(self.players == 1),
            mode: 0,
            bgm,
            time_limit,
            announcer: save.rules[2],
            stage: self.stage.stage,
            stage_change: save.rules[5] ^ 1,
            sides: std::array::from_fn(|i| SideSetup {
                control: control[i],
                pad: pad[i],
                change_allowed: change[i],
                chara: self.side[i].pick.picked,
                costume: self.side[i].pick.color_plate,
                cpu_level,
                items: self.side[i].pick.items,
                team_items: [[0; 8]; 4],
                team: [(0, 0); 4],
                team_count: 0,
                switch_enabled: 1,
                members: None,
            }),
            pool: Vec::new(),
            tutorial: 0,
        }
    }

    /// `CharSel_Update`.
    fn update(&mut self) {
        for f in &mut self.flash {
            f.advance();
        }
        for p in &mut self.panels {
            p.update();
        }
        for i in 0..2 {
            if self.side[i].mask != 0 && self.flash[1 + i].trig() & 1 != 0 {
                self.side[i].mask = 0;
            }
        }
        if self.stage.mask != 0 && self.flash[0].trig() & 1 != 0 {
            self.stage.mask = 0;
        }
        if self.face_mask != 0 && (self.flash[0].trig() as i32) < 0 {
            self.face_mask = 0;
        }
    }

    /// `CharSel_Draw`.
    fn draw(&mut self, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        for i in 0..2 {
            if self.flags & FLAG_STAGE_READY != 0 {
                if i == 0 {
                    self.bg_alpha[0] = (self.bg_alpha[0] + 5).min(0x80);
                } else {
                    self.bg_alpha[1] = (self.bg_alpha[1] - 5).max(0);
                }
            } else if i == 0 {
                self.bg_alpha[0] = 0;
                continue;
            } else {
                self.bg_alpha[1] = 0x80;
            }
            if let Some(first) = self.bg[i] {
                let alpha = self.bg_alpha[i] as u8;
                quads.extend(picture_quads(first).into_iter().map(|mut q| {
                    q.rgba[3] = alpha;
                    ScreenQuad::from(q)
                }));
            }
        }

        let mut texts: Vec<TextDraw> = Vec::new();
        for i in 0..2 {
            let lr = if i != 0 { 'r' } else { 'l' };
            let mask = self.face_mask != 0;
            self.set_flags(
                0,
                None,
                &format!("mc_face_mask_{lr}"),
                PROP_MASK_SET | PROP_VISIBLE,
                mask,
            );
            if self.side[i].flags & SIDE_FACE_READY != 0 {
                self.face_alpha[i] = (self.face_alpha[i] + 0.075).min(1.0);
            } else {
                self.face_alpha[i] = 0.0;
            }
            let chara = self.find(0, None, &format!("mc_single_chara_{lr}"));
            self.flash[0].clip_set_alpha(chara, self.face_alpha[i]);
            self.flash[0].clip_set_flags(chara, PROP_MASK, mask);
            let line = self.side[i].chara;
            let name = self.find(0, None, &format!("mc_name_text_{lr}"));
            let form = self.find(0, None, &format!("mc_form_text_{lr}"));
            let f = &mut self.flash[0];
            texts.extend(TextBox::preset(i as u32 + 1).attach_line(
                f,
                name,
                i as u32,
                &self.name_text,
                line,
                font,
            ));
            texts.extend(TextBox::preset(i as u32 + 3).attach_line(
                f,
                form,
                2 + i as u32,
                &self.form_text,
                line,
                font,
            ));
        }

        let stage = self.stage.stage;
        let map_name = Uv {
            u0: 0,
            v0: (stage % 4) * 0x40,
            u1: 0x200,
            v1: (stage % 4) * 0x40 + 0x40,
            tex: 0,
        };
        let clip = self.find(0, None, "mc_map_name");
        self.flash[0].clip_set_uv(clip, map_name);
        self.flash[0].clip_set_tex(clip, stage / 4);
        let mask = self.stage.mask != 0;
        self.set_flags(0, None, "mc_map_mask", PROP_MASK_SET | PROP_VISIBLE, mask);
        for i in 0..12 {
            self.set_flags(0, None, &format!("mc_map_chip_{i:02}"), PROP_MASK, mask);
        }
        for (i, (parent, icon)) in [
            ("mc_yajirusi_up", "mc_yajirusi_icon_up"),
            ("mc_yajirusi_down", "mc_yajirusi_icon_down"),
        ]
        .into_iter()
        .enumerate()
        {
            let clip = self.find(0, Some(parent), icon);
            if self.stage_rows >= 2 || self.stage.state == ST_BGM {
                let x = i as i32 * 0x20;
                self.flash[0].clip_set_uv(
                    clip,
                    Uv {
                        u0: x,
                        v0: 0x20,
                        u1: x + 0x20,
                        v1: 0x40,
                        tex: 0,
                    },
                );
                self.flash[0].clip_set_flags(clip, PROP_VISIBLE, true);
            } else {
                self.flash[0].clip_set_flags(clip, PROP_VISIBLE, false);
            }
        }
        let bgm = self.bgm_ids[self.stage.bgm as usize];
        let uv = Uv {
            u0: 0,
            v0: (bgm % 8) * 0x20,
            u1: 0x200,
            v1: (bgm % 8) * 0x20 + 0x20,
            tex: 0,
        };
        for child in ["mc_bgm_now_text_off", "mc_bgm_now_text_on"] {
            let clip = self.find(0, Some("mc_bgm_now"), child);
            self.flash[0].clip_set_uv(clip, uv);
            self.flash[0].clip_set_tex(clip, bgm / 8);
        }

        for i in 0..2 {
            let f = 3 + i;
            let x = (i as i32 ^ 1) * 0x20;
            self.set_uv(
                f,
                None,
                "mc_yajirusi",
                Uv {
                    u0: x,
                    v0: 0,
                    u1: x + 0x20,
                    v1: 0x20,
                    tex: 0,
                },
            );
            for j in 0..4 {
                let uv = Uv {
                    u0: 0,
                    v0: j * 0x20,
                    u1: 0x100,
                    v1: j * 0x20 + 0x20,
                    tex: 0,
                };
                let name = format!("mc_custom_plate_{}", j + 1);
                let plate = self.find(f, None, &name);
                self.flash[f]
                    .clip_set_color(plate, if j < self.custom_rows[i] { 1.0 } else { 0.3 });
                self.set_uv(f, Some(&name), "mc_custom_text_off", uv);
                self.set_uv(f, Some(&name), "mc_custom_text_on", uv);
                let arrow = j != 0 && self.cell(i).id != ID_RANDOM;
                self.set_flags(f, Some(&name), "mc_yajirusi", PROP_VISIBLE, arrow);
            }
            for j in 0..4 {
                let uv = Uv {
                    u0: (j % 2) * 0x40,
                    v0: (j / 2) * 0x20,
                    u1: (j % 2) * 0x40 + 0x40,
                    v1: (j / 2) * 0x20 + 0x20,
                    tex: 0,
                };
                let name = format!("mc_color_plate_{}", j + 1);
                let plate = self.find(f, None, &name);
                if j < self.color_count[i] {
                    self.flash[f].clip_set_flags(plate, PROP_VISIBLE, true);
                    match self.color_count[i] {
                        2 => self.flash[f].clip_set_offset(plate, 0x2D, 0),
                        3 => self.flash[f].clip_set_offset(plate, 0x16, 0),
                        _ => {}
                    }
                    self.set_uv(f, Some(&name), "mc_color_text_off", uv);
                    self.set_uv(f, Some(&name), "mc_color_text_on", uv);
                } else {
                    self.flash[f].clip_set_flags(plate, PROP_VISIBLE, false);
                }
            }
        }

        for i in 0..2 {
            let f = 1 + i;
            let plate = match self.players {
                0 => {
                    if i == 0 {
                        0
                    } else {
                        2
                    }
                }
                1 => i as i32,
                _ => 2,
            };
            let uv = Uv {
                u0: (plate % 2) * 0x40,
                v0: (plate / 2) * 0x20,
                u1: (plate % 2) * 0x40 + 0x40,
                v1: (plate / 2) * 0x20 + 0x20,
                tex: 0,
            };
            self.set_uv(f, None, "mc_plate_text", uv);
            let arrows = [
                ("mc_yajirusi_up", "mc_yajirusi_icon_up"),
                ("mc_yajirusi_down", "mc_yajirusi_icon_down"),
            ];
            if self.rows[i] >= 2 {
                for (j, (parent, icon)) in arrows.into_iter().enumerate() {
                    let x = j as i32 * 0x20;
                    self.set_uv(
                        f,
                        Some(parent),
                        icon,
                        Uv {
                            u0: x,
                            v0: 0x20,
                            u1: x + 0x20,
                            v1: 0x40,
                            tex: 0,
                        },
                    );
                    let show = if self.side[i].flags & SIDE_FORMS != 0 {
                        false
                    } else if i == 1 && self.players != 1 {
                        self.side[0].state == ST_DONE
                    } else {
                        true
                    };
                    self.set_flags(f, None, parent, PROP_VISIBLE, show);
                }
            } else {
                for (parent, _) in arrows {
                    self.set_flags(f, None, parent, PROP_VISIBLE, false);
                }
            }
            let mask = self.side[i].mask != 0;
            self.set_flags(f, None, "mc_chara_mask", PROP_MASK_SET | PROP_VISIBLE, mask);
            for j in 0..14 {
                self.set_flags(f, None, &format!("mc_chara_chip_{j:03}"), PROP_MASK, mask);
            }
        }

        for (f, flash) in self.flash.iter().enumerate() {
            let textures: &[Option<usize>] = match f {
                0 => &self.tex,
                1 | 2 => &self.side_tex[f - 1],
                _ => &self.plate_tex[f - 3],
            };
            let has_texture = |image: usize| textures.get(image).is_some_and(Option::is_some);
            let (flash_quads, overs) = flash.draw_with_overs(&has_texture);
            quads.extend(screen_quads(&flash_quads, textures));
            if f == 0 {
                draw_texts(&texts, &overs, font);
            }
        }
        font.flush(quads);
        self.icon.draw(quads);
        for p in &mut self.panels {
            p.draw(&self.chr, font, quads);
        }
        self.help.draw(self.item_info, &self.chr, font, quads);
    }

    /// `IconWin_SetIcon(icon)`: texture `icon + 1` of the icon file.
    fn set_icon(&mut self, icon: usize) {
        self.icon
            .set_texture(4, Some(self.layout.icons + 2 * (icon + 1)));
    }

    /// `CharSel_Input`.
    fn input(
        &mut self,
        pad: [(u32, u32); 2],
        save: &SaveData,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) {
        if self.flash[0].flags() & FLASH_PAD == 0 {
            return;
        }
        let count = if self.players == 1 { 2 } else { 1 };
        if self.flags & FLAG_STARTED == 0 {
            for i in 0..count {
                self.clip_goto(i + 1, i, Clip::Chip, "fl_on_start");
            }
            self.flags |= FLAG_STARTED;
        }
        let se = |sounds: &mut Vec<TitleSound>, bank: u32, id: u32| {
            sounds.push(TitleSound::Effect(bank, id))
        };
        if self.flags & FLAG_STAGE_PHASE != 0 {
            self.input_stage(pad[0], sounds);
            return;
        }
        for i in 0..count {
            let n = i + self.turn;
            if self.side[n ^ 1].flags & SIDE_ITEMS != 0 {
                continue;
            }
            let (pressed, repeat) = pad[i];
            let reel = n + 1;
            let plate = n + 3;
            match self.side[n].state {
                ST_GRID => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                        let row = self.side[n].pick.row;
                        let mut col = self.side[n].pick.col;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_move_left(&self.cells[n], &mut col, row);
                        } else {
                            chr_grid_move_right(&self.cells[n], &mut col, row);
                        }
                        self.side[n].pick.col = col;
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.side[n].chara = self.side[n].chip[col as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        if self.rows[n] < 2 {
                            continue;
                        }
                        let up = repeat & PAD_UP != 0;
                        self.flash[reel]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                        let (mut col, mut row) = (self.side[n].pick.col, self.side[n].pick.row);
                        if up {
                            chr_grid_move_up(&self.cells[n], &mut col, &mut row, self.rows[n]);
                        } else {
                            chr_grid_move_down(&self.cells[n], &mut col, &mut row, self.rows[n]);
                        }
                        (self.side[n].pick.col, self.side[n].pick.row) = (col, row);
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.set_row_chips(n);
                        self.side[n].chara = self.side[n].chip[col as usize];
                        self.request_face(n);
                        se(sounds, 2, if up { 2 } else { 1 });
                    } else if pressed & PAD_CROSS != 0 {
                        let cell = self.cell(n);
                        if cell.id == ID_RANDOM {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::Chip, "fl_ok");
                            self.custom_rows[n] = 4;
                            self.color_count[n] = 2;
                            if self.side[n].pick.color_plate >= self.color_count[n] {
                                self.side[n].pick.color_plate = 0;
                            }
                            self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                            self.side[n].state = ST_ITEMSET;
                            se(sounds, 1, 1);
                        } else if cell.id == ID_CUSTOM {
                            if self.custom_count[n] != 0 {
                                let (mut col, mut row) =
                                    (self.side[n].pick.custom_col, self.side[n].pick.custom_row);
                                chr_grid_fix_cursor(&self.custom[n], &mut col, &mut row, 2);
                                (self.side[n].pick.custom_col, self.side[n].pick.custom_row) =
                                    (col, row);
                                self.side[n].flags |= SIDE_CUSTOM;
                                self.flash[reel].goto_label("fl_form", true);
                                self.side[n].mask = 1;
                                self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                                self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                                self.set_custom_chips(n);
                                self.side[n].chara = self.side[n].chip[col as usize];
                                self.request_face(n);
                                self.side[n].state = ST_CUSTOM;
                                se(sounds, 2, 0x29);
                            } else {
                                se(sounds, 1, 7);
                            }
                        } else if cell.form_count != 0 {
                            self.side[n].flags |= SIDE_FORMS;
                            if self.side[n].pick.form >= cell.form_count {
                                self.side[n].pick.form = 0;
                            }
                            self.flash[reel].goto_label("fl_form", true);
                            self.side[n].mask = 1;
                            self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
                            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                            self.set_cell_form_chips(n);
                            self.side[n].chara = self.side[n].chip[self.side[n].pick.form as usize];
                            self.request_face(n);
                            self.side[n].state = ST_FORM;
                            se(sounds, 2, 0x29);
                        } else {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::Chip, "fl_ok");
                            self.custom_rows[n] = 4;
                            self.color_count[n] = self.wrap_costume(n);
                            self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                            self.side[n].state = ST_ITEMSET;
                            se(sounds, 1, 1);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        let turn = self.turn;
                        if self.players != 1 && turn != 0 {
                            self.clip_goto(i + 2, i + turn, Clip::Chip, "fl_off_start");
                            self.turn = 0;
                            let n = i;
                            self.flash[n + 1].goto_label("fl_in", true);
                            if self.side[n].flags & SIDE_FORMS != 0 {
                                self.clip_goto(n + 1, n, Clip::FormChip, "fl_on_start");
                                self.side[n].state = ST_FORM;
                            } else if self.side[n].flags & SIDE_CUSTOM != 0 {
                                self.clip_goto(n + 1, n, Clip::CustomChip, "fl_on_start");
                                self.side[n].state = ST_CUSTOM;
                            } else {
                                self.clip_goto(n + 1, n, Clip::Chip, "fl_on_start");
                                self.side[n].state = ST_GRID;
                            }
                        } else {
                            self.fade = ColorFade::start_out(true, 20);
                            self.running = false;
                        }
                        se(sounds, 1, 2);
                    }
                }
                ST_FORM => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::FormChip, "fl_off_start");
                        let chips = self.side[n].chip;
                        let mut form = self.side[n].pick.form;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_prev_form(&chips, &mut form);
                        } else {
                            chr_grid_next_form(&chips, &mut form);
                        }
                        self.side[n].pick.form = form;
                        self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                        self.side[n].chara = chips[form as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if pressed & PAD_CROSS != 0 {
                        self.flash[plate].goto_label("fl_custom_in", true);
                        self.clip_goto(reel, n, Clip::FormChip, "fl_ok");
                        if self.side[n].flags & SIDE_CUSTOM != 0 {
                            self.custom_rows[n] = 2;
                            if self.side[n].pick.custom_plate >= self.custom_rows[n] {
                                self.side[n].pick.custom_plate = 0;
                            }
                        } else {
                            self.custom_rows[n] = 4;
                        }
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        self.color_count[n] = self.wrap_costume(n);
                        self.side[n].state = ST_ITEMSET;
                        se(sounds, 1, 1);
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.side[n].flags ^= SIDE_FORMS;
                        self.flash[reel].goto_label("fl_form", true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::FormChip, "fl_off_start");
                        if self.side[n].flags & SIDE_CUSTOM != 0 {
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                            self.set_custom_chips(n);
                            self.side[n].chara =
                                self.side[n].chip[self.side[n].pick.custom_col as usize];
                            self.request_face(n);
                            self.side[n].state = ST_CUSTOM;
                        } else {
                            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                            self.set_row_chips(n);
                            self.side[n].chara = self.side[n].chip[self.side[n].pick.col as usize];
                            self.request_face(n);
                            self.side[n].state = ST_GRID;
                        }
                        se(sounds, 2, 0x29);
                    }
                }
                ST_ITEMSET => {
                    if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
                        let rows = self.custom_rows[n];
                        let p = &mut self.side[n].pick.custom_plate;
                        *p = if repeat & PAD_UP != 0 {
                            if *p - 1 < 0 { rows - 1 } else { *p - 1 }
                        } else if *p + 1 < rows {
                            *p + 1
                        } else {
                            0
                        };
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        se(sounds, 1, 0);
                    } else if pressed & if n == 0 { PAD_RIGHT } else { PAD_LEFT } != 0 {
                        // Towards the middle of the screen: look at the item set.
                        let plate = self.side[n].pick.custom_plate;
                        if plate <= 0 || self.cell(n).id == ID_RANDOM {
                            continue;
                        }
                        self.clip_goto(n + 3, n, Clip::CustomPlate, "fl_ok");
                        let p = self.side[n].pick;
                        let chara = self.side[n].chara;
                        if self.cell(n).id == ID_CUSTOM {
                            let slot = (p.custom_col + p.custom_row * 7) as usize;
                            let items = save.rec_items.get(slot).copied().unwrap_or_default();
                            let level = save.rec_level.get(slot).copied().unwrap_or(0);
                            self.panels[n].set_chara(
                                chara,
                                items,
                                level.into(),
                                0,
                                true,
                                &self.chr,
                            );
                        } else {
                            let slot = (p.col + p.row * 7) as usize;
                            let set = (plate - 1) as usize;
                            let items = save.custom_items.get(slot).map_or([0; 8], |s| s[set]);
                            let level = save.custom_level.get(slot).copied().unwrap_or(0);
                            self.panels[n].set_chara(
                                chara,
                                items,
                                level.into(),
                                plate - 1,
                                false,
                                &self.chr,
                            );
                        }
                        self.panels[n].show(sounds);
                        self.side[n].flags |= SIDE_ITEMS;
                        self.side[n].state = ST_ITEMS;
                    } else if pressed & PAD_CROSS != 0 {
                        self.flash[plate].goto_label("fl_color_in", true);
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_ok");
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_on_start");
                        self.side[n].state = ST_COLOR;
                        se(sounds, 1, 1);
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.flash[plate].goto_label("fl_custom_cansel", true);
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
                        self.back_to_reel(n);
                        se(sounds, 1, 2);
                    }
                }
                ST_ITEMS => {
                    let r = self.panels[n].input(pressed, repeat, sounds);
                    if r > 0 {
                        self.help.open();
                        self.item_info = (r - 1) as usize;
                        self.side[n].state = ST_ITEMINFO;
                    } else if r < 0 {
                        self.panels[n].hide(sounds);
                        self.side[n].flags &= !SIDE_ITEMS;
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        self.side[n].state = ST_ITEMSET;
                    }
                }
                ST_ITEMINFO => {
                    if pressed & (PAD_CROSS | PAD_TRIANGLE) != 0 {
                        self.help.close();
                        self.side[n].state = ST_ITEMS;
                        se(sounds, 1, 2);
                    }
                }
                ST_COLOR => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_off_start");
                        let count = self.color_count[n];
                        let c = &mut self.side[n].pick.color_plate;
                        *c = if repeat & PAD_LEFT != 0 {
                            if *c - 1 < 0 { count - 1 } else { *c - 1 }
                        } else if *c + 1 < count {
                            *c + 1
                        } else {
                            0
                        };
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_on_start");
                        se(sounds, 1, 0);
                    } else if pressed & PAD_CROSS != 0 {
                        self.confirm_color(n, i, save, random);
                        se(sounds, 1, 1);
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.flash[plate].goto_label("fl_color_cansel", true);
                        self.clip_goto(plate, n, Clip::ColorPlate, "fl_off_start");
                        self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                        self.side[n].state = ST_ITEMSET;
                        se(sounds, 1, 2);
                    }
                }
                ST_CUSTOM => {
                    if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        let row = self.side[n].pick.custom_row;
                        let mut col = self.side[n].pick.custom_col;
                        if repeat & PAD_LEFT != 0 {
                            chr_grid_move_left(&self.custom[n], &mut col, row);
                        } else {
                            chr_grid_move_right(&self.custom[n], &mut col, row);
                        }
                        self.side[n].pick.custom_col = col;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                        self.side[n].chara = self.side[n].chip[col as usize];
                        self.request_face(n);
                        se(sounds, 2, 0);
                    } else if repeat & (PAD_UP | PAD_DOWN) != 0 {
                        let up = repeat & PAD_UP != 0;
                        self.flash[reel]
                            .goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        let (mut col, mut row) =
                            (self.side[n].pick.custom_col, self.side[n].pick.custom_row);
                        if up {
                            chr_grid_move_up(&self.custom[n], &mut col, &mut row, 2);
                        } else {
                            chr_grid_move_down(&self.custom[n], &mut col, &mut row, 2);
                        }
                        (self.side[n].pick.custom_col, self.side[n].pick.custom_row) = (col, row);
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
                        self.set_custom_chips(n);
                        self.side[n].chara = self.side[n].chip[col as usize];
                        self.request_face(n);
                        se(sounds, 2, if up { 2 } else { 1 });
                    } else if pressed & PAD_CROSS != 0 {
                        let cell = self.custom[n][self.custom_index(n)];
                        if cell.form_count != 0 {
                            self.side[n].flags |= SIDE_FORMS;
                            if self.side[n].pick.form >= cell.form_count {
                                self.side[n].pick.form = 0;
                            }
                            self.flash[reel].goto_label("fl_form", true);
                            self.side[n].mask = 1;
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
                            self.set_form_chips(n);
                            self.side[n].chara = self.side[n].chip[self.side[n].pick.form as usize];
                            self.request_face(n);
                            self.side[n].state = ST_FORM;
                            se(sounds, 2, 0x29);
                        } else {
                            self.flash[plate].goto_label("fl_custom_in", true);
                            self.clip_goto(reel, n, Clip::CustomChip, "fl_ok");
                            self.custom_rows[n] = 2;
                            if self.side[n].pick.custom_plate >= self.custom_rows[n] {
                                self.side[n].pick.custom_plate = 0;
                            }
                            self.clip_goto(plate, n, Clip::CustomPlate, "fl_on_start");
                            self.color_count[n] = self.wrap_costume(n);
                            self.side[n].state = ST_ITEMSET;
                            se(sounds, 1, 1);
                        }
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.side[n].flags ^= SIDE_CUSTOM;
                        self.flash[reel].goto_label("fl_form", true);
                        self.side[n].mask = 1;
                        self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.set_row_chips(n);
                        self.side[n].chara = self.side[n].chip[self.side[n].pick.col as usize];
                        self.request_face(n);
                        self.side[n].state = ST_GRID;
                        se(sounds, 2, 0x29);
                    }
                }
                ST_DONE => {
                    if self.flags & FLAG_STAGE_PHASE != 0 {
                        continue;
                    }
                    if self.side[n ^ 1].state == ST_DONE {
                        self.flash[0].goto_label("fl_map", true);
                        self.icon.open();
                        self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                        self.flags |= FLAG_STAGE_PHASE;
                        self.stage.state = ST_STAGE;
                        self.face_mask = 1;
                    } else if pressed & PAD_TRIANGLE != 0 {
                        self.flash[reel].goto_label("fl_in", true);
                        self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
                        self.set_row_chips(n);
                        self.side[n].chara = self.side[n].chip[self.side[n].pick.col as usize];
                        self.request_face(n);
                        self.side[n].state = ST_GRID;
                        se(sounds, 1, 2);
                    }
                }
                _ => {}
            }
        }
    }

    /// The reel state a side's flags point back to (forms, saved characters or the grid).
    fn back_to_reel(&mut self, n: usize) {
        let reel = n + 1;
        if self.side[n].flags & SIDE_FORMS != 0 {
            self.clip_goto(reel, n, Clip::FormChip, "fl_on_start");
            self.side[n].state = ST_FORM;
        } else if self.side[n].flags & SIDE_CUSTOM != 0 {
            self.clip_goto(reel, n, Clip::CustomChip, "fl_on_start");
            self.side[n].state = ST_CUSTOM;
        } else {
            self.clip_goto(reel, n, Clip::Chip, "fl_on_start");
            self.side[n].state = ST_GRID;
        }
    }

    /// `ChrTbl_WrapCostume(chara, &colorPlate)`.
    fn wrap_costume(&mut self, n: usize) -> i32 {
        let count = self
            .chr
            .costumes
            .get(self.side[n].chara as usize)
            .copied()
            .unwrap_or(1);
        if self.side[n].pick.color_plate >= count {
            self.side[n].pick.color_plate = 0;
        }
        count
    }

    /// Cross on the costume: the side's character is final (the random cell draws one now).
    fn confirm_color(&mut self, n: usize, i: usize, save: &SaveData, random: &mut GameRandom) {
        let (reel, plate) = (n + 1, n + 3);
        self.side[n].mask = 0;
        self.flash[plate].goto_label("fl_color_ok", true);
        self.flash[reel].goto_label("fl_out", true);
        self.clip_goto(plate, n, Clip::ColorPlate, "fl_ok");
        self.clip_goto(plate, n, Clip::CustomPlate, "fl_off_start");
        // The items of the chosen set (plate 0: none): a saved custom character's, else the character slot's set.
        let s = self.side[n];
        let set = (s.pick.custom_plate - 1).max(0) as usize;
        let rec = |slot: i32| {
            save.rec_items
                .get(slot as usize)
                .copied()
                .unwrap_or_default()
        };
        let custom = |slot: usize| save.custom_items.get(slot).map_or([0; 8], |c| c[set]);
        let custom_slot = s.pick.custom_col + s.pick.custom_row * 7;
        let has_set = s.pick.custom_plate != 0;
        if s.flags & SIDE_FORMS != 0 {
            self.side[n].pick.picked = s.chip[s.pick.form as usize];
            self.side[n].pick.items = match has_set {
                false => [0; 8],
                true if self.cell(n).id == ID_CUSTOM => rec(custom_slot),
                true => custom((s.pick.col + s.pick.row * 7) as usize),
            };
            self.clip_goto(reel, n, Clip::FormChip, "fl_off_start");
        } else if s.flags & SIDE_CUSTOM != 0 {
            self.side[n].pick.picked = s.chip[s.pick.custom_col as usize];
            self.side[n].pick.items = if has_set { rec(custom_slot) } else { [0; 8] };
            self.clip_goto(reel, n, Clip::CustomChip, "fl_off_start");
        } else {
            let mut slot = (s.pick.col + s.pick.row * 7) as usize;
            if self.cell(n).id == ID_RANDOM {
                let cells = &self.cells[n];
                let k = loop {
                    let k = random.range(self.master_count[n] as u32) as usize;
                    if cells[k].id < ID_RANDOM {
                        break k;
                    }
                };
                let mut chara = cells[k].id;
                if cells[k].form_count != 0 {
                    chara = cells[k].form[random.range(cells[k].form_count as u32) as usize];
                }
                self.side[n].pick.picked = chara;
                slot = k;
            } else {
                self.side[n].pick.picked = self.side[n].chip[self.side[n].pick.col as usize];
            }
            self.side[n].pick.items = if has_set { custom(slot) } else { [0; 8] };
            self.clip_goto(reel, n, Clip::Chip, "fl_off_start");
        }
        self.side[n].state = ST_DONE;
        if self.players != 1 && self.turn == 0 {
            self.turn = 1;
            let n = i + self.turn;
            self.clip_goto(n + 1, n, Clip::Chip, "fl_on_start");
            self.side[n].state = ST_GRID;
        }
    }

    /// The stage and music choice of pad 0.
    fn input_stage(&mut self, (pressed, repeat): (u32, u32), sounds: &mut Vec<TitleSound>) {
        let se = |sounds: &mut Vec<TitleSound>, bank: u32, id: u32| {
            sounds.push(TitleSound::Effect(bank, id))
        };
        match self.stage.state {
            ST_STAGE => {
                let on_music = self.stage.col == STAGE_BGM_COL;
                if repeat & (PAD_LEFT | PAD_RIGHT) != 0 {
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let mut col = self.stage.col;
                    if repeat & PAD_LEFT != 0 {
                        stg_grid_move_left(&self.stage_ids, &mut col, self.stage.row);
                    } else {
                        stg_grid_move_right(&self.stage_ids, &mut col, self.stage.row);
                    }
                    self.stage.col = col;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    if col != STAGE_BGM_COL {
                        self.stage.stage = self.stage_ids[(col + self.stage.row * 6) as usize];
                        self.request_stage();
                    }
                    se(sounds, 2, 0);
                } else if repeat & (PAD_UP | PAD_DOWN) != 0 && !on_music {
                    if self.stage_rows < 2 {
                        return;
                    }
                    let up = repeat & PAD_UP != 0;
                    self.flash[0].goto_label(if up { "fl_reel_down" } else { "fl_reel_up" }, true);
                    self.stage.mask = 1;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let (mut col, mut row) = (self.stage.col, self.stage.row);
                    if up {
                        stg_grid_move_up(&self.stage_ids, &mut col, &mut row, self.stage_rows);
                    } else {
                        stg_grid_move_down(&self.stage_ids, &mut col, &mut row, self.stage_rows);
                    }
                    (self.stage.col, self.stage.row) = (col, row);
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    self.set_stage_chips();
                    self.stage.stage = self.stage_ids[(col + row * 6) as usize];
                    self.request_stage();
                    se(sounds, 2, if up { 2 } else { 1 });
                } else if pressed & PAD_CROSS != 0 {
                    self.stage.mask = 0;
                    self.clip_goto(0, 0, Clip::StageChip, "fl_ok");
                    if on_music {
                        self.flash[0].goto_label("fl_bgm", true);
                        self.set_icon(1);
                        self.stage.bgm = self.stage.bgm_playing;
                        self.stage.state = ST_BGM;
                    } else {
                        self.flash[0].goto_label("fl_vs", true);
                        self.icon.close();
                        self.face_mask = 0;
                        self.flags |= FLAG_LEAVING | FLAG_10;
                        self.timer = 60;
                    }
                    se(sounds, 1, 1);
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.stage.mask = 0;
                    self.flags ^= FLAG_STAGE_PHASE;
                    self.flash[0].goto_label("fl_map_cansel", true);
                    self.icon.close();
                    self.clip_goto(0, 0, Clip::StageChip, "fl_off_start");
                    let sides: &[usize] = if self.players != 1 { &[1] } else { &[0, 1] };
                    for &j in sides {
                        self.flash[1 + j].goto_label("fl_in", true);
                        // A saved custom character goes back to the form state, as in the original.
                        if self.side[j].flags & SIDE_FORMS != 0 {
                            self.clip_goto(j + 1, j, Clip::FormChip, "fl_on_start");
                        } else if self.side[j].flags & SIDE_CUSTOM != 0 {
                            self.clip_goto(j + 1, j, Clip::CustomChip, "fl_on_start");
                        } else {
                            self.clip_goto(j + 1, j, Clip::Chip, "fl_on_start");
                            self.side[j].state = ST_GRID;
                            continue;
                        }
                        self.side[j].state = ST_FORM;
                    }
                    se(sounds, 1, 2);
                }
            }
            ST_BGM => {
                if repeat & PAD_UP != 0 {
                    self.stage.bgm -= 1;
                    if self.stage.bgm < 0 {
                        self.stage.bgm = self.bgm_count - 1;
                    }
                    se(sounds, 1, 0);
                } else if repeat & PAD_DOWN != 0 {
                    self.stage.bgm += 1;
                    if self.stage.bgm > self.bgm_count - 1 {
                        self.stage.bgm = 0;
                    }
                    se(sounds, 1, 0);
                } else if pressed & PAD_CROSS != 0 {
                    let id = self.bgm_ids[self.stage.bgm as usize];
                    if id != BGM_LOCKED {
                        self.flash[0].goto_label("fl_bgm_cansel", true);
                        self.set_icon(0);
                        self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                        self.stage.bgm_playing = self.stage.bgm;
                        if id != BGM_RANDOM {
                            sounds.push(TitleSound::Bgm(BGM_FIRST + id as u32));
                        }
                        self.stage.state = ST_STAGE;
                        se(sounds, 1, 1);
                    } else {
                        se(sounds, 1, 7);
                    }
                } else if pressed & PAD_TRIANGLE != 0 {
                    self.flash[0].goto_label("fl_bgm_cansel", true);
                    self.set_icon(0);
                    self.clip_goto(0, 0, Clip::StageChip, "fl_on_start");
                    self.stage.bgm = self.stage.bgm_playing;
                    self.stage.state = ST_STAGE;
                    se(sounds, 1, 2);
                }
            }
            _ => {}
        }
    }
}

/// `ChrGrid_IsSelectable`: a character, the custom cell or the random cell.
pub(crate) fn chr_grid_is_selectable(cells: &[Cell], index: usize) -> bool {
    cells
        .get(index)
        .is_some_and(|c| c.id < ID_LOCKED || c.id == ID_CUSTOM)
}

fn cell_id(cells: &[Cell], col: i32, row: i32) -> i32 {
    cells
        .get((row * COLS as i32 + col) as usize)
        .map_or(ID_EMPTY, |c| c.id)
}

/// `ChrGrid_MoveRight`: the next selectable cell to the right, wrapping; false if there is no other one.
pub(crate) fn chr_grid_move_right(cells: &[Cell], col: &mut i32, row: i32) -> bool {
    let start = *col;
    loop {
        *col += 1;
        if *col >= COLS as i32 {
            *col = 0;
        }
        if start == *col {
            return false;
        }
        let id = cell_id(cells, *col, row);
        if id < ID_LOCKED || id == ID_CUSTOM {
            return true;
        }
    }
}

/// `ChrGrid_MoveLeft`.
pub(crate) fn chr_grid_move_left(cells: &[Cell], col: &mut i32, row: i32) -> bool {
    let start = *col;
    loop {
        *col -= 1;
        if *col < 0 {
            *col = COLS as i32 - 1;
        }
        if start == *col {
            return false;
        }
        let id = cell_id(cells, *col, row);
        if id < ID_LOCKED || id == ID_CUSTOM {
            return true;
        }
    }
}

/// `ChrGrid_MoveDown`: the next row (wrapping) with a selectable cell, sliding right in the row if needed.
pub(crate) fn chr_grid_move_down(cells: &[Cell], col: &mut i32, row: &mut i32, rows: i32) {
    loop {
        *row += 1;
        if *row > rows - 1 {
            *row = 0;
        }
        let id = cell_id(cells, *col, *row);
        if id < ID_LOCKED || id == ID_CUSTOM || chr_grid_move_right(cells, col, *row) {
            return;
        }
    }
}

/// `ChrGrid_MoveUp`.
pub(crate) fn chr_grid_move_up(cells: &[Cell], col: &mut i32, row: &mut i32, rows: i32) {
    loop {
        *row -= 1;
        if *row < 0 {
            *row = rows - 1;
        }
        let id = cell_id(cells, *col, *row);
        if id < ID_LOCKED || id == ID_CUSTOM || chr_grid_move_right(cells, col, *row) {
            return;
        }
    }
}

/// `ChrGrid_NextForm`: the next of the seven chips that is a character.
pub(crate) fn chr_grid_next_form(forms: &[i32; 7], index: &mut i32) {
    let start = *index;
    loop {
        *index += 1;
        if *index >= FORM_MAX as i32 {
            *index = 0;
        }
        if start == *index || forms[*index as usize] < ID_RANDOM {
            return;
        }
    }
}

/// `ChrGrid_PrevForm`.
pub(crate) fn chr_grid_prev_form(forms: &[i32; 7], index: &mut i32) {
    let start = *index;
    loop {
        *index -= 1;
        if *index < 0 {
            *index = FORM_MAX as i32 - 1;
        }
        if start == *index || forms[*index as usize] < ID_RANDOM {
            return;
        }
    }
}

/// `ChrGrid_FixCursor`: off a marker cell, right in the row first, then down.
pub(crate) fn chr_grid_fix_cursor(cells: &[Cell], col: &mut i32, row: &mut i32, rows: i32) {
    if cell_id(cells, *col, *row) >= ID_RANDOM && !chr_grid_move_right(cells, col, *row) {
        chr_grid_move_down(cells, col, row, rows);
    }
}

fn stage_id(ids: &[i32], col: i32, row: i32) -> i32 {
    ids.get((row * STAGE_COLS + col) as usize)
        .copied()
        .unwrap_or(STAGE_LOCKED)
}

/// `StgGrid_MoveRight`: stops in column 6, the music entry.
pub(crate) fn stg_grid_move_right(ids: &[i32], col: &mut i32, row: i32) {
    loop {
        *col += 1;
        if *col >= 7 {
            *col = 0;
        } else if *col == STAGE_COLS {
            return;
        }
        if stage_id(ids, *col, row) < STAGE_LOCKED {
            return;
        }
    }
}

/// `StgGrid_MoveLeft`: from column 0 to column 6.
pub(crate) fn stg_grid_move_left(ids: &[i32], col: &mut i32, row: i32) {
    loop {
        *col -= 1;
        if *col < 0 {
            *col = STAGE_COLS;
            return;
        }
        if stage_id(ids, *col, row) < STAGE_LOCKED {
            return;
        }
    }
}

/// `StgGrid_MoveRightWrap`.
fn stg_grid_move_right_wrap(ids: &[i32], col: &mut i32, row: i32) -> bool {
    let start = *col;
    loop {
        *col += 1;
        if *col >= STAGE_COLS {
            *col = 0;
        }
        if start == *col {
            return false;
        }
        if stage_id(ids, *col, row) < STAGE_LOCKED {
            return true;
        }
    }
}

/// `StgGrid_MoveDown`.
pub(crate) fn stg_grid_move_down(ids: &[i32], col: &mut i32, row: &mut i32, rows: i32) {
    loop {
        *row += 1;
        if *row > rows - 1 {
            *row = 0;
        }
        if stage_id(ids, *col, *row) < STAGE_LOCKED || stg_grid_move_right_wrap(ids, col, *row) {
            return;
        }
    }
}

/// `StgGrid_MoveUp`.
pub(crate) fn stg_grid_move_up(ids: &[i32], col: &mut i32, row: &mut i32, rows: i32) {
    loop {
        *row -= 1;
        if *row < 0 {
            *row = rows - 1;
        }
        if stage_id(ids, *col, *row) < STAGE_LOCKED || stg_grid_move_right_wrap(ids, col, *row) {
            return;
        }
    }
}

/// `StgGrid_ApplyUnlocks` in mode 39: a locked stage becomes `STGGRID_ID_LOCKED`; the random entry stays.
pub(crate) fn stage_grid_apply_unlocks(ids: &mut [i32], save: &SaveData) {
    for id in ids.iter_mut() {
        if *id != STAGE_RANDOM && !save.stage_unlocked(*id) {
            *id = STAGE_LOCKED;
        }
    }
}

/// `BgmList_ApplyUnlocks` outside mode 0x3E: an entry without its bit becomes `BGMLIST_ID_LOCKED`.
pub(crate) fn bgm_list_apply_unlocks(ids: &mut [i32], save: &SaveData) {
    for id in ids.iter_mut() {
        if *id != BGM_RANDOM && !save.bgm_unlocked(*id) {
            *id = BGM_LOCKED;
        }
    }
}

/// `ChrGrid_Build` in mode 39: locked characters become `LOCKED`, a cell keeps its unlocked forms, the grid is padded
/// to whole rows; then the list of saved custom characters.
pub(crate) fn chr_grid_build(master: &[Cell], save: &SaveData) -> (Vec<Cell>, Vec<Cell>, i32) {
    chr_grid_build_with(master, save, 0)
}

/// `CHRGRID_NO_CUSTOM`: the custom character cell is empty (and no list of them is made).
pub(crate) const GRID_NO_CUSTOM: u32 = 1;
/// `CHRGRID_NO_RANDOM`: the random cell is empty.
pub(crate) const GRID_NO_RANDOM: u32 = 2;
/// `CHRGRID_NO_FORMS`: a cell of forms shows its first one only.
pub(crate) const GRID_NO_FORMS: u32 = 4;

/// `ChrGrid_Build` with the flags its progress mode gives (`GRID_*`): modes 33..35 have no custom cell, 13..30 no
/// custom nor random cell (21 no forms either), 48..50 none of the three.
pub(crate) fn chr_grid_build_with(
    master: &[Cell],
    save: &SaveData,
    flags: u32,
) -> (Vec<Cell>, Vec<Cell>, i32) {
    let mut out = Vec::new();
    for cell in master {
        if cell.form_count != 0 {
            let mut c = Cell::default();
            for &form in &cell.form[..cell.form_count.clamp(0, FORM_MAX as i32) as usize] {
                if save.chara_unlocked(form) {
                    c.form[c.form_count as usize] = form;
                    c.form_count += 1;
                }
            }
            match c.form_count {
                0 => c.id = ID_LOCKED,
                1 => {
                    c.form_count = 0;
                    c.id = c.form[0];
                }
                _ => {
                    if flags & GRID_NO_FORMS != 0 {
                        c.form_count = 0;
                    }
                    c.id = c.form[0];
                }
            }
            out.push(c);
        } else {
            match cell.id {
                ID_CUSTOM if flags & GRID_NO_CUSTOM != 0 => out.push(Cell {
                    id: ID_EMPTY,
                    ..Cell::default()
                }),
                ID_RANDOM if flags & GRID_NO_RANDOM != 0 => out.push(Cell {
                    id: ID_EMPTY,
                    ..Cell::default()
                }),
                ID_CUSTOM | ID_RANDOM => out.push(*cell),
                id if save.chara_unlocked(id) => out.push(*cell),
                _ => out.push(Cell {
                    id: ID_LOCKED,
                    ..*cell
                }),
            }
        }
    }
    while out.len() % COLS != 0 {
        out.push(Cell {
            id: ID_EMPTY,
            ..Cell::default()
        });
    }
    let mut custom = vec![
        Cell {
            id: ID_LOCKED,
            ..Cell::default()
        };
        CUSTOM_MAX
    ];
    let mut count = 0;
    if flags & GRID_NO_CUSTOM != 0 {
        return (out, custom, count);
    }
    for (i, &chara) in save.rec_chara.iter().enumerate() {
        if chara < 0 {
            continue;
        }
        if let Some(k) = master.iter().position(|c| c.id == chara) {
            custom[i] = out[k];
            custom[i].id = chara;
            custom[i].form[0] = chara;
            count += 1;
        }
    }
    (out, custom, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoPictures;

    impl PictureLoader for NoPictures {
        fn portrait(&mut self, _: u32) -> Option<usize> {
            Some(1000)
        }
        fn picture(&mut self, _: u32) -> Option<usize> {
            Some(2000)
        }
    }

    #[test]
    fn grid_moves_skip_locked_cells_and_wrap() {
        let cell = |id| Cell {
            id,
            ..Cell::default()
        };
        let cells: Vec<Cell> = [0, ID_LOCKED, 2, ID_EMPTY, ID_CUSTOM, 5, ID_RANDOM]
            .into_iter()
            .map(cell)
            .collect();
        let mut col = 0;
        assert!(chr_grid_move_right(&cells, &mut col, 0));
        assert_eq!(col, 2);
        chr_grid_move_right(&cells, &mut col, 0);
        assert_eq!(col, 4);
        let mut col = 0;
        chr_grid_move_left(&cells, &mut col, 0);
        assert_eq!(col, 6);
        let ids = [0, 1, STAGE_LOCKED, 3, 4, 5];
        let mut col = 1;
        stg_grid_move_right(&ids, &mut col, 0);
        assert_eq!(col, 3);
        let mut col = 5;
        stg_grid_move_right(&ids, &mut col, 0);
        assert_eq!(col, STAGE_BGM_COL);
    }

    /// Runs the real character select from the ISO in `rom/`, when it is there: 1P vs COM, a character for each
    /// side, then the stage.
    #[test]
    fn real_char_select_sets_up_a_battle() {
        use crate::duel_menu::ARCHIVE_FILE_ID;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            eprintln!("skipped: no ISO at {}", iso.display());
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, ARCHIVE_SECTION).unwrap()).unwrap();
        let assets = CharSelAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = CharSelLayout::new(|key| firsts[&key]);
        println!(
            "{} cells, stages {:X?}, music {:X?}",
            data.master.len(),
            data.stage_ids,
            data.bgm_ids
        );
        // Side 0's character (grid slot 1) has items in its first set.
        let mut save = SaveData::default();
        save.custom_items[1][0] = [3, 7, 0, 0, 0, 0, 0, 0];
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[3, 1, 4, 1]);
        let mut sounds = Vec::new();
        let mut screen = CharSelScreen::new(
            &data,
            0,
            CharSelProgress::default(),
            &save,
            ChrTable {
                costumes: vec![2; 0xA1],
                ..ChrTable::default()
            },
            &mut NoPictures,
            &mut random,
            &mut sounds,
        )
        .unwrap();
        println!(
            "rows {:?}, charas {:#X} / {:#X}, start sounds {sounds:?}",
            screen.rows, screen.side[0].chara, screen.side[1].chara
        );
        macro_rules! step {
            ($bits:expr) => {
                screen.run_frame(
                    [($bits, $bits), (0, 0)],
                    &mut NoPictures,
                    &save,
                    &mut random,
                    &mut font,
                )
            };
        }
        macro_rules! wait {
            () => {
                for _ in 0..40 {
                    step!(0);
                }
            };
        }
        wait!();
        let quads = step!(0).quads.len();
        println!("{quads} quads on screen");
        assert!(quads > 50);
        // Side 0: one to the right, then confirm until the costume is taken; on the item sets, set 1 is looked at
        // in the panel and its first item in the details page.
        step!(PAD_RIGHT);
        let mut looked = false;
        for _ in 0..4 {
            if screen.side[0].state == ST_ITEMSET && !looked {
                looked = true;
                let before = step!(0).quads.len();
                step!(PAD_DOWN);
                wait!();
                assert_eq!(screen.side[0].pick.custom_plate, 1);
                step!(PAD_RIGHT);
                wait!();
                assert_eq!(screen.side[0].state, ST_ITEMS);
                let shown = step!(0).quads.len();
                assert!(shown > before + 20, "{before} -> {shown}");
                step!(PAD_CROSS);
                wait!();
                assert_eq!((screen.side[0].state, screen.item_info), (ST_ITEMINFO, 2));
                step!(PAD_TRIANGLE);
                wait!();
                assert_eq!(screen.side[0].state, ST_ITEMS);
                step!(PAD_TRIANGLE);
                wait!();
                assert_eq!(screen.side[0].state, ST_ITEMSET);
            }
            let state = screen.side[0].state;
            step!(PAD_CROSS);
            wait!();
            println!(
                "side 0: state {state} -> {}, chara {:#X}",
                screen.side[0].state, screen.side[0].chara
            );
            if screen.side[0].state == ST_DONE {
                break;
            }
        }
        assert_eq!(screen.side[0].state, ST_DONE);
        assert_eq!(screen.turn, 1);
        for _ in 0..4 {
            step!(PAD_CROSS);
            wait!();
            if screen.side[1].state == ST_DONE {
                break;
            }
        }
        assert_eq!(screen.side[1].state, ST_DONE);
        wait!();
        assert_eq!(screen.stage.state, ST_STAGE);
        step!(PAD_CROSS);
        for _ in 0..120 {
            step!(0);
        }
        let Some(CharSelResult::Battle(setup)) = screen.result() else {
            panic!("no battle: {:?}", screen.result());
        };
        println!("{setup:?}");
        assert_eq!(setup.sides[0].control, 0);
        assert_eq!(setup.sides[1].control, 2);
        assert_eq!(setup.sides[1].cpu_level, 13);
        assert_eq!(setup.time_limit, 4);
        assert_eq!(screen.progress().side[0].picked, setup.sides[0].chara);
        assert!(looked);
        assert_eq!(setup.sides[0].items, [3, 7, 0, 0, 0, 0, 0, 0]);
    }

    /// The training's character select from the ISO in `rom/` (`CharSel_Run(2)` in mode 0x2D, menu archive 6),
    /// when it is there: confirm through both sides and the stage, and free training is set up.
    #[test]
    fn real_training_char_select_sets_up_free_training() {
        use crate::training::{ARCHIVE_FILE_ID, CHAR_SELECT_SECTION};
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, CHAR_SELECT_SECTION).unwrap()).unwrap();
        let assets = CharSelAssets::parse(&res).unwrap();
        let mut next = 10;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, textures) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * textures.len();
        }
        let mut data = assets.data;
        data.layout = CharSelLayout::new(|key| firsts[&key]);
        let save = SaveData::default();
        let mut font = Font::new(Vec::new(), 20);
        let mut random = GameRandom::from_key(&[2, 7, 1]);
        let mut screen = CharSelScreen::new(
            &data,
            0,
            CharSelProgress::default(),
            &save,
            ChrTable {
                costumes: vec![2; 0xA1],
                ..ChrTable::default()
            },
            &mut NoPictures,
            &mut random,
            &mut Vec::new(),
        )
        .unwrap()
        .for_training();
        for _ in 0..30 {
            if screen.result().is_some() {
                break;
            }
            screen.run_frame(
                [(PAD_CROSS, PAD_CROSS), (0, 0)],
                &mut NoPictures,
                &save,
                &mut random,
                &mut font,
            );
            for _ in 0..60 {
                screen.run_frame([(0, 0); 2], &mut NoPictures, &save, &mut random, &mut font);
            }
        }
        let Some(CharSelResult::Battle(setup)) = screen.result() else {
            panic!("no battle: {:?}", screen.result());
        };
        assert_eq!(
            (
                setup.mode,
                setup.announcer,
                setup.time_limit,
                setup.stage_change
            ),
            (6, 1, 0, 0)
        );
        assert_eq!(
            (
                setup.sides[0].control,
                setup.sides[1].control,
                setup.sides[1].cpu_level
            ),
            (0, 2, -1)
        );
    }

    /// `BattleSetup_FixForMode`: each mode's sides and rule.
    #[test]
    fn finish_applies_the_mode_rules() {
        let versus = |mode| {
            let mut s = BattleSetup {
                mode,
                screen_mode: 1,
                time_limit: 2,
                ..BattleSetup::default()
            };
            s.sides[0].team_count = 2;
            s.sides[1].team_count = 2;
            s.sides[1].control = 2;
            s.sides[1].cpu_level = 13;
            s.finish();
            s
        };
        let s = versus(0);
        assert_eq!((s.screen_mode, s.sides[0].team_count), (1, 2));
        let s = versus(4);
        assert_eq!(
            (s.screen_mode, s.sides[0].team_count, s.sides[1].team_count),
            (1, 0, 0)
        );
        let s = versus(6);
        assert_eq!(s.screen_mode, 0);
        assert_eq!((s.sides[1].control, s.sides[1].cpu_level), (2, -1));
        let s = versus(7);
        assert_eq!((s.time_limit, s.sides[0].control), (5, 2));
        let s = versus(9);
        assert_eq!((s.mode, s.time_limit), (6, 0));
        assert_eq!((s.sides[0].control, s.sides[0].cpu_level), (2, 29));
        // Side 1 was the computer already: its level stays.
        assert_eq!(s.sides[1].cpu_level, 13);
        let mut s = versus(3);
        assert_eq!((s.sides[0].team_count, s.sides[1].team_count), (0, 2));
        s.pool = vec![PoolMember {
            chara: 7,
            costume: 1,
            cpu_level: 21,
            items: [5, 0, 0, 0, 0, 0, 0, 0],
        }];
        s.finish();
        assert_eq!(
            (s.sides[1].chara, s.sides[1].cpu_level, s.sides[1].items[0]),
            (7, 21, 5)
        );
    }

    /// The survival courses from the ISO in `rom/`, when it is there: course 0's fifty opponents, the first of
    /// them side 1's fighter.
    #[test]
    fn real_survival_courses_set_up_the_pool() {
        use crate::duel_menu::ARCHIVE_FILE_ID;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let location = partition_entry_for_id(ARCHIVE_FILE_ID + 3);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let pack = archive.read_entry(&mut image, location.index).unwrap();
        let res = bpe::decode(pack_section(&pack, 2).unwrap()).unwrap();
        let data = bt3_data::survival::SurvivalData::parse(&res).unwrap();
        for c in &data.courses {
            println!(
                "course: announcer {} stage {} bgm {} time {} change {}/{}",
                c.announcer, c.stage, c.bgm, c.time_limit, c.stage_change, c.change_allowed
            );
        }
        let mut random = GameRandom::from_key(&[1]);
        let mut libc = crate::rng::LibcRandom::new(1);
        let setup =
            BattleSetup::survival(&data, 0, (3, 0, [0; 8]), &mut random, &mut libc).unwrap();
        println!(
            "pool {:?}",
            setup.pool.iter().map(|m| m.chara).collect::<Vec<_>>()
        );
        assert_eq!(setup.mode, 3);
        assert_eq!(setup.pool.len(), bt3_data::survival::OPPONENTS);
        assert_eq!(setup.sides[0].chara, 3);
        assert_eq!((setup.sides[0].control, setup.sides[1].control), (0, 2));
        assert_eq!(setup.sides[1].chara, setup.pool[0].chara);
        assert!(setup.pool.iter().all(|m| (0..0xA0).contains(&m.chara)));
        assert!((0..35).contains(&setup.stage));
        assert!(BattleSetup::survival(&data, 3, (3, 0, [0; 8]), &mut random, &mut libc).is_none());
    }

    /// `Demo_SetupBattle`: a computer pairing on its stage, never the last one twice in a row.
    #[test]
    fn demo_battles_change_their_pairing() {
        let mut random = GameRandom::from_key(&[7]);
        let mut last = -1;
        let mut seen = Vec::new();
        for _ in 0..40 {
            let before = last;
            let s = BattleSetup::demo(&mut random, &mut last);
            assert_ne!(last, before);
            let (a, b, stage) = DEMO_PAIRS[last as usize];
            assert_eq!((s.sides[0].chara, s.sides[1].chara, s.stage), (a, b, stage));
            assert_eq!(
                (s.mode, s.time_limit, s.announcer, s.screen_mode),
                (7, 5, 4, 0)
            );
            assert_eq!((s.sides[0].control, s.sides[1].control), (2, 2));
            assert_eq!((s.sides[0].cpu_level, s.sides[1].pad), (13, 1));
            assert!((8..17).contains(&s.bgm));
            seen.push(last);
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 9);
    }
}
