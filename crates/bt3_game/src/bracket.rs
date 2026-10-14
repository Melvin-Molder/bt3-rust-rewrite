//! Dragon World Tour's bracket (`Bracket_Run`, src/menu/bracket*.c, progress mode 35): over the tournament's
//! backdrop, the tree of seventeen entrants (sixteen in a knock-out, the winner meeting the seeded boss), the
//! guide announcing each round and each match, the two chips meeting and the versus panel before a battle, the
//! winner's chip moving up after it, the CPU-only matches decided by a coin toss, and at the end the banner of a
//! win or a loss, the prizes of the open tournament (money, an item, characters, a stage, maybe a dragon ball) and
//! the system save.

use std::error::Error;

use bt3_data::chara_data::ChrTable;
use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{
    BattleSetup, Cell, GRID_NO_CUSTOM, MemberStart, PictureLoader, SideSetup, chr_grid_build_with,
};
use crate::dialog::{Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::get_win::{self, GetWin, GetWinData};
use crate::logo::ColorFade;
use crate::mcflow::{self, Done, McEnv, McFlow, McIo};
use crate::menu_res::{FileKey, ImageTexture, decode_files, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::title::{PAD_CROSS, PAD_LEFT, PAD_RIGHT, PadRepeat, TitleFrame, TitleSound};
use crate::tour_bg::TourBg;
use crate::tour_menu::{
    ENT_BOSS, ENT_HIDDEN, ENT_LOST, ENT_PLAYER, ENT_REC, ENTRANT_MAX, TourEntrant, TourSession,
    VOICE_BASE,
};
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// The bracket's pack: file `gProgress->baseFile + 6 + tournament`.
pub const PACK_FILE_FIRST: u32 = 0x1C1 + 6;
const FACE_FILE: u32 = 0x2F9;
const FADE_FRAMES: i32 = 0x14;
const CHIP_COUNT: usize = 165;
/// `TOUR_MATCH_MAX` and `LTOUR_ROUND_NUM`.
pub const MATCH_MAX: usize = 16;
const ROUND_NUM: i32 = 5;
/// The entrants a real character may be drawn among (`LCHR_ID_MAX`).
const CHARA_ID_MAX: i32 = 0xA0;

// BRK_BGM_* and the opening jingle.
const BGM_WORLD: u32 = 0x10B29;
const BGM_CELL: u32 = 0x10B24;
const BGM_OTHER: u32 = 0x10B18;
const SE_FANFARE: u32 = 0x10BCA;

// TOUR_*.
const WORLD: i32 = 0;
const BIG: i32 = 1;
const CELL: i32 = 2;
const OTHERWORLD: i32 = 3;
const YAMCHA: i32 = 4;

// flash[].
const FL_TREE: usize = 0;
const FL_MOVE_A: usize = 1;
const FL_MOVE_B: usize = 2;
const FL_GUIDE: usize = 3;
const FL_VS: usize = 4;
const FL_TITLE: usize = 5;
/// `Flash.flags`: playing, at its end.
const FLASH_PLAYING: u32 = 1;
const FLASH_END: u32 = 8;

// BRK_*.
const DONE: u32 = 1;
const LEAVING: u32 = 2;
const INPUT: u32 = 4;
const NO_TEXT: u32 = 8;
const RESULT: u32 = 0x10;
const MOVE_B: u32 = 0x20;
const MOVE_A: u32 = 0x40;
const BUSY: u32 = 0x80;
const RESULT0: u32 = 0x100;
const IMAGES: u32 = 0x200;
const SCROLL_BACK: u32 = 0x400;
const RUNNER_UP: u32 = 0x1000;
const PLAYER_OUT: u32 = 0x4000;
const VS_FRONT: u32 = 0x40000;
const FANFARE: u32 = 0x80000;

// TOUR_MATCH_*.
pub const MATCH_DONE: u16 = 1;
const MATCH_ROUND_END: u16 = 2;
const MATCH_TO_LEFT: u16 = 4;
const MATCH_TO_RIGHT: u16 = 8;
const MATCH_FINAL: u16 = 0x10;

// LBRK_LOAD_*.
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;

// Sequence steps that start a result.
const SEQ_ROUND_END: i32 = 0x65;

// LPRIZE_*.
const PRIZE_MONEY_WIN: usize = 0;
const PRIZE_MONEY_SECOND: usize = 1;
const PRIZE_ITEM: usize = 2;
const PRIZE_CHARA_A: usize = 3;
const PRIZE_CHARA_B: usize = 4;
const PRIZE_STAGE: usize = 5;
const PRIZE_NUM: usize = 7;
const REWARD_MAX: usize = 8;

/// One match of the bracket (`TourMatch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TourMatch {
    /// `TOUR_MATCH_*`.
    pub flags: u16,
    /// The entrants, left and right.
    pub ent: [u16; 2],
    /// The match the winner goes to.
    pub next: u16,
    /// The tree position the winner's chip moves to.
    pub pos: u16,
    pub vs_anim: u16,
    pub view: u16,
    pub win_anim: u16,
}

/// How the battle of the current match ended (`BattleResult`): the winner bits (1 side 0, 2 side 1) and each
/// side's team health in percent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BattleOutcome {
    pub winner: u32,
    pub health: [f32; 2],
}

/// Where `Bracket_Run` leads.
#[derive(Clone, Debug, PartialEq)]
pub enum BracketResult {
    /// 1: the battle of the current match (`Bracket_SetupBattle`).
    Battle(Box<BattleSetup>),
    /// 0: the tournament is over.
    Over,
}

/// `texTitle[n]` of `Bracket_Load` (flash 5).
const TEX_TITLE: [ImageTexture; 38] = [
    tex(1, 0),
    tex(28, 4),
    tex(28, 0),
    tex(28, 5),
    tex(28, 1),
    tex(28, 6),
    tex(28, 2),
    tex(28, 7),
    tex(28, 3),
    tex(5, 0),
    None,
    None,
    None,
    None,
    None,
    tex(35, 1),
    tex(37, 0),
    tex(35, 0),
    tex(32, 0),
    tex(34, 2),
    tex(34, 0),
    tex(34, 3),
    tex(34, 1),
    tex(3, 0),
    tex(3, 1),
    tex(28, 8),
    tex(3, 2),
    tex(2, 2),
    tex(2, 0),
    tex(2, 3),
    tex(2, 1),
    tex(3, 3),
    tex(27, 1),
    tex(6, 1),
    tex(2, 5),
    tex(2, 4),
    tex(27, 0),
    tex(6, 0),
];
/// The three title textures the backdrop's movie takes too (`bgTex`).
const BG_EXTRA: [ImageTexture; 3] = [tex(3, 3), tex(28, 8), tex(3, 2)];
/// `texTree[n]` (flash 0); 6 and 9..24 are the entrants' chips.
const TEX_TREE: [ImageTexture; 26] = [
    tex(30, 0),
    tex(30, 1),
    tex(30, 2),
    tex(30, 3),
    tex(9, 1),
    tex(9, 0),
    None,
    tex(8, 0),
    tex(8, 1),
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
    None,
    None,
    None,
    None,
    None,
    None,
    tex(7, 0),
];
/// `Bracket_SetChipTex`: the tree's image of each entrant's chip.
const CHIP_IMAGES: [usize; ENTRANT_MAX] = [
    6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
];
/// `texMoveA[n]` / `texMoveB[n]` (flashes 1 and 2); 1 and 4 are the two chips.
const TEX_MOVE_A: [ImageTexture; 6] = [tex(11, 0), None, tex(8, 0), tex(8, 1), None, tex(12, 0)];
const TEX_MOVE_B: [ImageTexture; 6] = [tex(11, 0), None, tex(8, 0), tex(8, 1), None, tex(11, 1)];
/// `texVs[n]` (flash 4); 5 and 6 are the two fighters' large pictures.
const TEX_VS: [ImageTexture; 16] = [
    tex(15, 0),
    tex(19, 0),
    tex(17, 0),
    tex(20, 0),
    tex(19, 1),
    None,
    None,
    tex(18, 0),
    tex(18, 2),
    tex(18, 1),
    tex(18, 3),
    tex(15, 1),
    None,
    None,
    None,
    None,
];
/// `MsgWin_Init(MPACK_AT(res, 26), …)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((26, 1), 0)), Some(((26, 1), 1)), None];
const GET_WIN_PACK: usize = 42;
const DIALOG_SECTION: usize = 36;
/// The guide's two picture sets (sections 43 and 44): images 0, 2, 1 or 3, 5, 4 take entries 0, 1, 3.
const POSE_TEXTURES: [[ImageTexture; 3]; 2] = [
    [tex(43, 0), tex(43, 1), tex(43, 3)],
    [tex(44, 0), tex(44, 1), tex(44, 3)],
];

/// `texGuide[n]` of `Bracket_Load` by tournament.
fn guide_textures(tour: i32) -> [ImageTexture; 9] {
    let mut t = [None; 9];
    match tour {
        BIG | CELL | YAMCHA => {
            t[0] = tex(43, 0);
            t[2] = tex(43, 1);
            t[1] = tex(43, 3);
            if tour == YAMCHA {
                t[6] = tex(45, 0);
                t[8] = tex(45, 1);
                t[7] = tex(45, 3);
            }
        }
        _ => {
            t[0] = tex(43, 0);
            t[1] = tex(43, 1);
        }
    }
    t
}

/// Whether the tournament's guide has the two picture sets (`guideTex`).
fn has_poses(tour: i32) -> bool {
    matches!(tour, BIG | CELL | YAMCHA)
}

fn chip_textures() -> Vec<ImageTexture> {
    (1..=CHIP_COUNT).map(|n| Some(((23, n), 0))).collect()
}

/// `LTourCpu`: the computer's strength in a round, `[level][round]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TourCpu {
    pub cpu_level: i32,
    /// Item ids - 1 (0xFFFF: none).
    pub items: [u16; 8],
}

/// What `Bracket_Load` reads from a tournament's pack.
pub struct BracketAssets {
    pub data: BracketData,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct BracketData {
    /// The movies: tree 10, chips 13 and 14, guide 29, versus panel 16, title 4.
    pub fods: [Vec<u8>; 6],
    pub msg_fod: Vec<u8>,
    /// Section 25: the guide's lines; 39: their mouth keys; 21 / 22: character and form names.
    pub msg_text: Vec<u8>,
    pub lips: Vec<u8>,
    pub names: Vec<u8>,
    pub forms: Vec<u8>,
    /// Section 24: the character grid.
    pub(crate) master: Vec<Cell>,
    /// Section 31: `[level * 5 + round]`.
    pub cpu: Vec<TourCpu>,
    /// Section 41: per tournament, `prize[kind][level]` (-1 none).
    pub prizes: Vec<[[i32; 3]; PRIZE_NUM]>,
    pub get_win: GetWinData,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub layout: BracketLayout,
}

#[derive(Clone, Debug, Default)]
pub struct BracketLayout {
    pub title: Vec<Option<usize>>,
    pub tree: Vec<Option<usize>>,
    pub move_a: Vec<Option<usize>>,
    pub move_b: Vec<Option<usize>>,
    pub vs: Vec<Option<usize>>,
    pub guide: Vec<Option<usize>>,
    /// The guide's two picture sets, when it has them.
    pub poses: Option<[Vec<Option<usize>>; 2]>,
    pub msg: Vec<Option<usize>>,
    pub get_win: Vec<Option<usize>>,
    pub chips: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
    /// `TourBg_Init`'s three textures of the title movie.
    pub bg_extra: [Option<usize>; 3],
}

impl BracketLayout {
    pub fn new(tour: i32, dialog: Vec<Option<usize>>, first: impl Fn(FileKey) -> usize) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        let extra = i(&BG_EXTRA);
        Self {
            title: i(&TEX_TITLE),
            tree: i(&TEX_TREE),
            move_a: i(&TEX_MOVE_A),
            move_b: i(&TEX_MOVE_B),
            vs: i(&TEX_VS),
            guide: i(&guide_textures(tour)),
            poses: has_poses(tour).then(|| [i(&POSE_TEXTURES[0]), i(&POSE_TEXTURES[1])]),
            msg: i(&MSG_TEXTURES),
            get_win: i(&get_win::textures(GET_WIN_PACK)),
            chips: i(&chip_textures()),
            dialog,
            bg_extra: [extra[0], extra[1], extra[2]],
        }
    }
}

impl BracketAssets {
    /// `Bracket_Load` for tournament `tour` from its unpacked pack.
    pub fn parse(res: &[u8], tour: i32) -> Result<Self, Box<dyn Error>> {
        let guide = guide_textures(tour);
        let chips = chip_textures();
        let gw = get_win::textures(GET_WIN_PACK);
        let mut tables: Vec<&[ImageTexture]> = vec![
            &TEX_TITLE,
            &TEX_TREE,
            &TEX_MOVE_A,
            &TEX_MOVE_B,
            &TEX_VS,
            &guide,
            &MSG_TEXTURES,
            &gw,
            &chips,
        ];
        if has_poses(tour) {
            tables.extend(POSE_TEXTURES.iter().map(|p| &p[..]));
        }
        let files = decode_files(res, tables)?;
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let word = |data: &[u8], at: usize| -> Result<i32, Box<dyn Error>> {
            Ok(i32::from_le_bytes(
                data.get(at..at + 4).ok_or("short section")?.try_into()?,
            ))
        };
        let grid = pack_section(res, 24)?;
        let master = (0..word(grid, 0)?.max(0) as usize)
            .map(|i| {
                let at = 0x10 + i * 0x24;
                Ok(Cell {
                    id: word(grid, at)?,
                    form_count: word(grid, at + 4)?,
                    form: std::array::from_fn(|k| word(grid, at + 8 + k * 4).unwrap_or(0)),
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let cpu_sec = pack_section(res, 31)?;
        let cpu = (0..3 * ROUND_NUM as usize)
            .map(|n| {
                let at = n * 0x14;
                let mut items = [0u16; 8];
                for (k, item) in items.iter_mut().enumerate() {
                    let b = cpu_sec
                        .get(at + 4 + k * 2..at + 6 + k * 2)
                        .ok_or("short CPU table")?;
                    *item = u16::from_le_bytes(b.try_into()?);
                }
                Ok(TourCpu {
                    cpu_level: word(cpu_sec, at)?,
                    items,
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let prize_sec = pack_section(res, 41)?;
        let prizes = (0..5)
            .map(|t| {
                let mut p = [[0; 3]; PRIZE_NUM];
                for (kind, row) in p.iter_mut().enumerate() {
                    for (level, v) in row.iter_mut().enumerate() {
                        *v = word(prize_sec, t * 0x64 + 0x10 + kind * 12 + level * 4)?;
                    }
                }
                Ok(p)
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let dialog = DialogAssets::parse(pack_section(res, DIALOG_SECTION)?)?;
        Ok(Self {
            data: BracketData {
                fods: [sec(10)?, sec(13)?, sec(14)?, sec(29)?, sec(16)?, sec(4)?],
                msg_fod: pack_section(pack_section(res, 26)?, 2)?.to_vec(),
                msg_text: sec(25)?,
                lips: sec(39)?,
                names: sec(21)?,
                forms: sec(22)?,
                master,
                cpu,
                prizes,
                get_win: GetWinData::parse_lang(pack_section(res, GET_WIN_PACK)?, 1)?,
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                layout: BracketLayout::default(),
            },
            files,
            dialog,
        })
    }
}

/// `Bracket_GetViewX`: how far the tree is scrolled to show a match.
fn view_x(view: u16) -> i32 {
    const X: [i32; 12] = [
        0, 0x50, 0xC4, 0x138, 0x1AC, 0x200, 0x8A, 0x172, 0x16, 0x1E4, 0x100, 0x32,
    ];
    X.get(usize::from(view)).copied().unwrap_or(0)
}

/// `Bracket_GetPosX`: x of a tree position (0..15 the leaves, 16..29 the nodes, 30 the boss, 31 the winner of
/// match 14, 32 the champion).
fn pos_x(pos: i32) -> i32 {
    const X: [i32; 33] = [
        0x33, 0x6D, 0xA7, 0xE1, 0x11B, 0x155, 0x18F, 0x1C9, 0x203, 0x23D, 0x277, 0x2B1, 0x2EB,
        0x325, 0x35F, 0x399, 0x50, 0xC4, 0x137, 0x1AB, 0x21F, 0x293, 0x306, 0x37A, 0x8A, 0x171,
        0x259, 0x340, 0xFD, 0x2CC, 0x54, 0x1E5, 0x11E,
    ];
    usize::try_from(pos)
        .ok()
        .and_then(|p| X.get(p).copied())
        .unwrap_or(0)
}

/// `Bracket_GetPosY`: one height per round.
fn pos_y(pos: i32) -> i32 {
    match pos {
        0..=15 => 0x10D,
        16..=23 => 0xDC,
        24..=27 => 0xC9,
        28..=29 => 0xA7,
        30..=31 => 0x89,
        32 => 0x67,
        _ => 0,
    }
}

/// `Bracket_InitMatches`: the fixed table of the sixteen matches.
fn init_matches() -> [TourMatch; MATCH_MAX] {
    // (flags, next, pos, vsAnim, view, winAnim) of each match.
    const L: u16 = MATCH_TO_LEFT;
    const R: u16 = MATCH_TO_RIGHT;
    const E: u16 = MATCH_ROUND_END;
    const T: [(u16, u16, u16, u16, u16, u16); MATCH_MAX] = [
        (L, 8, 16, 0, 0, 0),
        (R, 8, 17, 0, 0, 0),
        (L, 9, 18, 0, 1, 0),
        (R, 9, 19, 0, 2, 0),
        (L, 10, 20, 0, 3, 0),
        (R, 10, 21, 0, 4, 0),
        (L, 11, 22, 0, 5, 0),
        (R | E, 11, 23, 0, 5, 0),
        (L, 12, 24, 1, 0, 1),
        (R, 12, 25, 1, 6, 1),
        (L, 13, 26, 1, 7, 1),
        (R | E, 13, 27, 1, 5, 1),
        (L, 14, 28, 2, 8, 2),
        (R | E, 14, 29, 2, 9, 2),
        (R | E, 15, 31, 3, 10, 3),
        (MATCH_FINAL | E, 0, 32, 4, 11, 4),
    ];
    let mut m = [TourMatch::default(); MATCH_MAX];
    for (i, (flags, next, pos, vs_anim, view, win_anim)) in T.into_iter().enumerate() {
        m[i] = TourMatch {
            flags,
            ent: if i < 8 {
                [2 * i as u16, 2 * i as u16 + 1]
            } else {
                [0, 0]
            },
            next,
            pos,
            vs_anim,
            view,
            win_anim,
        };
    }
    m[15].ent[0] = 16;
    m
}

/// Which pictures the guide shows for a subtitle line (`Bracket_SetTalker`): the picture set (pose) and, in the
/// Yamcha Game, which of the two guides talks.
fn talker_for(line: i32) -> (Option<usize>, Option<usize>) {
    match line {
        // Cell Games.
        0x4D..=0x54 | 0x56..=0x5C | 0x5F..=0x79 => (Some(0), None),
        0x55 | 0x5D | 0x5E => (Some(1), None),
        // Big Tournament.
        0x7D | 0x91..=0x98 | 0x9B | 0x9D | 0x9E => (Some(0), None),
        0x99 | 0x9A | 0x9C => (Some(1), None),
        // Yamcha Game.
        0xD1 | 0xD7 => (None, Some(0)),
        0xCD..=0xCF | 0xD2 | 0xD3 | 0xD5 | 0xD6 | 0xD8..=0xE6 => (Some(0), Some(1)),
        0xD0 | 0xD4 | 0xF5 => (Some(1), Some(1)),
        _ => (None, None),
    }
}

/// `Bracket`.
pub struct BracketScreen {
    data: BracketData,
    bg: TourBg,
    flash: [Flash; 6],
    tex: [Vec<Option<usize>>; 6],
    msg: MsgWin,
    get_win: GetWin,
    dialog: Dialog,
    mcflow: McFlow,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    session: TourSession,
    grid: Vec<Cell>,
    flags: u32,
    voice_line: i32,
    win_side: usize,
    result: i32,
    pose: usize,
    talker: usize,
    timer: i32,
    started: bool,
    seq: i32,
    load_state: i32,
    loading: [Option<usize>; 2],
    round: i32,
    match_no: usize,
    mc_state: i32,
    scroll_speed: i32,
    scroll: f32,
    blink: [i32; 2],
    talk: [i32; 2],
    ents: [TourEntrant; ENTRANT_MAX],
    matches: [TourMatch; MATCH_MAX],
    reward: Vec<(i32, i32)>,
    reward_idx: usize,
    done: Option<BracketResult>,
}

impl BracketScreen {
    /// `Bracket_Init` and the first `ColorFade_StartIn`: a new tournament draws its entrants and plays the
    /// jingle; one that runs takes over the battle's `outcome`, decides the CPU matches of the round and starts
    /// the music.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &BracketData,
        bg: TourBg,
        session: &TourSession,
        outcome: BattleOutcome,
        chr: &ChrTable,
        save: &SaveData,
        card_res: Option<&[u8]>,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        sounds: &mut Vec<TitleSound>,
    ) -> Result<Self, Box<dyn Error>> {
        let running = session.running;
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> { Ok(Flash::create(fod)?) };
        let mut flash = [
            make(&data.fods[0])?,
            make(&data.fods[1])?,
            make(&data.fods[2])?,
            make(&data.fods[3])?,
            make(&data.fods[4])?,
            make(&data.fods[5])?,
        ];
        // Bracket_Load: the title plays (its opening with a tournament running); the guide and its window come
        // in for a new one, the versus panel for one that runs.
        flash[FL_TITLE].play(1);
        if running {
            flash[FL_TITLE].goto_label("fl_title_in", true);
        }
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            false,
        )?;
        if !running {
            flash[FL_GUIDE].play(1);
            flash[FL_GUIDE].goto_label("fl_guide_in", true);
            msg.open();
        } else {
            flash[FL_VS].play(1);
            flash[FL_VS].goto_label("fl_in", true);
        }
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            None,
            0,
        )?;
        let (grid, _, _) = chr_grid_build_with(&data.master, save, GRID_NO_CUSTOM);
        let mut screen = Self {
            data: data.clone(),
            bg,
            flash,
            tex: [
                data.layout.tree.clone(),
                data.layout.move_a.clone(),
                data.layout.move_b.clone(),
                data.layout.guide.clone(),
                data.layout.vs.clone(),
                data.layout.title.clone(),
            ],
            msg,
            get_win: GetWin::new(&data.get_win, data.layout.get_win.clone())?,
            dialog,
            mcflow: McFlow::new(card_res, None),
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            session: session.clone(),
            grid,
            flags: 0,
            voice_line: -1,
            win_side: 0,
            result: 1,
            pose: 0,
            talker: 0,
            timer: 0,
            started: false,
            seq: 0,
            load_state: 0,
            loading: [None; 2],
            round: 0,
            match_no: 0,
            mc_state: 0,
            scroll_speed: 0,
            scroll: 0.0,
            blink: [0; 2],
            talk: [0; 2],
            ents: session.entrants,
            matches: [TourMatch::default(); MATCH_MAX],
            reward: Vec::new(),
            reward_idx: 0,
            done: None,
        };
        if !running {
            // A new tournament.
            screen.session.matches = [TourMatch::default(); MATCH_MAX];
            screen.session.round = 0;
            screen.session.match_no = 0;
            screen.fill_entrants(chr, random);
            screen.shuffle_entrants(random);
            screen.matches = init_matches();
            sounds.push(TitleSound::StreamSe(SE_FANFARE));
            screen.flags |= FANFARE;
        } else {
            // Back from a battle.
            screen.round = session.round;
            screen.match_no = session.match_no.clamp(0, MATCH_MAX as i32 - 1) as usize;
            screen.matches = session.matches;
            screen.load_images(loader);
            screen.apply_result(outcome);
            screen.play_cpu_matches(random);
            sounds.push(TitleSound::Bgm(screen.bgm()));
        }
        // The matches no player entrant takes part in are skipped.
        while screen.match_no < MATCH_MAX - 1 && !screen.has_player(screen.match_no) {
            screen.match_no += 1;
        }
        screen.set_chip_tex();
        Ok(screen)
    }

    /// `Bracket_Run`'s result once it has returned.
    pub fn result(&self) -> Option<&BracketResult> {
        self.done.as_ref()
    }

    /// The tournament's block of `gProgress` as `Bracket_Run` leaves it.
    pub fn session(&self) -> TourSession {
        self.session.clone()
    }

    fn tour(&self) -> i32 {
        self.session.tour.clamp(0, 4)
    }

    fn bgm(&self) -> u32 {
        match self.tour() {
            WORLD => BGM_WORLD,
            CELL => BGM_CELL,
            _ => BGM_OTHER,
        }
    }

    fn cur(&self) -> TourMatch {
        self.matches[self.match_no.min(MATCH_MAX - 1)]
    }

    fn ent(&self, side: usize) -> &TourEntrant {
        &self.ents[usize::from(self.cur().ent[side]).min(ENTRANT_MAX - 1)]
    }

    fn ent_mut(&mut self, side: usize) -> &mut TourEntrant {
        let i = usize::from(self.cur().ent[side]).min(ENTRANT_MAX - 1);
        &mut self.ents[i]
    }

    fn has_player(&self, m: usize) -> bool {
        self.matches[m]
            .ent
            .iter()
            .any(|&e| self.ents[usize::from(e).min(ENTRANT_MAX - 1)].flags & ENT_PLAYER != 0)
    }

    fn chip(&self, chara: i32) -> Option<usize> {
        usize::try_from(chara)
            .ok()
            .and_then(|c| self.data.layout.chips.get(c).copied())
            .flatten()
    }

    fn cost(chr: &ChrTable, chara: i32) -> i32 {
        usize::try_from(chara)
            .ok()
            .and_then(|c| chr.costs.get(c).copied())
            .unwrap_or(0)
    }

    /// `Bracket_FillEntrants`: the players' entrants, then random characters of the grid whose cost fits the level
    /// (given up after fifteen misses in a row), none twice and never the boss, who is the seventeenth.
    fn fill_entrants(&mut self, chr: &ChrTable, random: &mut GameRandom) {
        let boss = match self.tour() {
            WORLD => {
                if random.range(2) != 0 {
                    0x42
                } else {
                    0x38
                }
            }
            BIG => 0x37,
            CELL => {
                if random.range(2) != 0 {
                    0x6B
                } else {
                    0x6C
                }
            }
            OTHERWORLD => {
                if random.range(2) != 0 {
                    0x3D
                } else {
                    6
                }
            }
            _ => 0x1A,
        };
        let level = self.session.level;
        let entry_num = self.session.entry_num.max(0) as usize;
        let mut miss = 0;
        let mut i = entry_num;
        while i < ENTRANT_MAX {
            let cell = loop {
                let n = random.range(self.grid.len() as u32) as usize;
                let cell = self.grid[n];
                if cell.id <= CHARA_ID_MAX {
                    break cell;
                }
            };
            let chara = if cell.form_count != 0 {
                cell.form[random.range(cell.form_count as u32) as usize]
            } else {
                cell.id
            };
            let cost = Self::cost(chr, chara);
            let unfit = match level {
                0 => cost >= 4,
                1 => cost >= 8,
                _ => cost < 7,
            };
            if unfit && miss < 15 {
                miss += 1;
                continue;
            }
            if self.ents[..i].iter().any(|e| e.chara == chara) || chara == boss {
                continue;
            }
            miss = 0;
            self.ents[i].chara = chara;
            i += 1;
        }
        let b = &mut self.ents[16];
        b.pos = 30;
        b.chara = boss;
        b.flags |= ENT_BOSS;
    }

    /// `Bracket_ShuffleEntrants`: the sixteen first-round entrants shuffled, a player to the left of a computer in
    /// each pair, the leaves numbered.
    fn shuffle_entrants(&mut self, random: &mut GameRandom) {
        for i in 0..16 {
            let r = random.range(16) as usize;
            self.ents.swap(r, i);
        }
        for i in (0..16).step_by(2) {
            if self.ents[i].flags & ENT_PLAYER == 0 && self.ents[i + 1].flags & ENT_PLAYER != 0 {
                self.ents.swap(i, i + 1);
            }
        }
        for (i, e) in self.ents.iter_mut().take(16).enumerate() {
            e.pos = i as i32;
        }
    }

    /// `Bracket_SetChipTex`.
    fn set_chip_tex(&mut self) {
        for (i, &image) in CHIP_IMAGES.iter().enumerate() {
            self.tex[FL_TREE][image] = self.chip(self.ents[i].chara);
        }
    }

    /// `Bracket_PlayCpuMatch`: a coin toss; the loser is marked, the winner takes the match's tree position and
    /// its slot in the next match (a side already marked lost moves on without a toss, as the original has it).
    fn play_cpu_match(&mut self, m: usize, random: &mut GameRandom) {
        let mt = self.matches[m];
        let e = mt.ent.map(|e| usize::from(e).min(ENTRANT_MAX - 1));
        if mt.flags & MATCH_DONE != 0
            || self.ents[e[0]].flags & ENT_PLAYER != 0
            || self.ents[e[1]].flags & ENT_PLAYER != 0
        {
            return;
        }
        let win = if self.ents[e[0]].flags & ENT_LOST != 0 {
            0
        } else if self.ents[e[1]].flags & ENT_LOST != 0 {
            1
        } else {
            let win = random.range(2) as usize;
            self.ents[e[win ^ 1]].flags |= ENT_LOST;
            win
        };
        self.ents[e[win]].pos = i32::from(mt.pos);
        let next = usize::from(mt.next).min(MATCH_MAX - 1);
        if mt.flags & MATCH_TO_LEFT != 0 {
            self.matches[next].ent[0] = mt.ent[win];
        } else if mt.flags & MATCH_TO_RIGHT != 0 {
            self.matches[next].ent[1] = mt.ent[win];
        }
        self.matches[m].flags |= MATCH_DONE;
    }

    /// `Bracket_PlayCpuMatches`: the computer-only matches in order, up to the end of the current round.
    fn play_cpu_matches(&mut self, random: &mut GameRandom) {
        let mut round = 0;
        for m in 0..MATCH_MAX {
            if !self.has_player(m) {
                self.play_cpu_match(m, random);
            }
            if self.matches[m].flags & MATCH_ROUND_END != 0 {
                round += 1;
                if self.round < round {
                    return;
                }
            }
        }
    }

    /// `Bracket_NextMatch`: on to the next match with a player in it; passing a round's last match announces the
    /// next round.
    fn next_match(&mut self) {
        loop {
            if self.cur().flags & MATCH_ROUND_END != 0 {
                self.session.greeted = false;
                self.round += 1;
                if self.round < ROUND_NUM {
                    self.seq = SEQ_ROUND_END;
                }
            }
            if self.match_no + 1 >= MATCH_MAX {
                break;
            }
            self.match_no += 1;
            if self.has_player(self.match_no) {
                break;
            }
        }
    }

    /// `Bracket_IsPlayerOut`.
    fn is_player_out(&self) -> bool {
        !self
            .ents
            .iter()
            .any(|e| e.flags & ENT_PLAYER != 0 && e.flags & ENT_LOST == 0)
    }

    /// `Bracket_ApplyResult`: the pad side is battle side 0 (the left entrant when it is a player, else the right
    /// one); the loser is marked only while `Bracket_IsPlayerOut` looks, the winner keeps its health.
    fn apply_result(&mut self, outcome: BattleOutcome) {
        let left_player = self.ent(0).flags & ENT_PLAYER != 0;
        let won = if outcome.winner & 1 != 0 {
            Some((if left_player { 0 } else { 1 }, outcome.health[0]))
        } else if outcome.winner & 2 != 0 {
            Some((if left_player { 1 } else { 0 }, outcome.health[1]))
        } else {
            None
        };
        if let Some((win, health)) = won {
            let lose = win ^ 1;
            self.ent_mut(lose).flags |= ENT_LOST;
            self.win_side = win;
            self.flags |= RESULT;
            if self.is_player_out() {
                self.flags |= PLAYER_OUT;
            }
            self.ent_mut(win).health = health;
            self.ent_mut(lose).flags ^= ENT_LOST;
        }
        self.ent_mut(0).flags &= !ENT_HIDDEN;
        self.ent_mut(1).flags &= !ENT_HIDDEN;
        let m = self.match_no;
        self.matches[m].flags |= MATCH_DONE;
    }

    /// `Bracket_StartVs`: the current match's two chips meet; their large pictures start loading.
    fn start_vs(&mut self) {
        self.load_state = LOAD_REQUEST;
        self.flags |= MOVE_A;
        self.tex[FL_MOVE_A][1] = self.chip(self.ent(0).chara);
        self.tex[FL_MOVE_A][4] = self.chip(self.ent(1).chara);
        self.scroll = -(view_x(self.cur().view) as f32);
        self.flash[FL_TREE].set_offset(self.scroll as i32, 0);
        self.ent_mut(0).flags |= ENT_HIDDEN;
        self.ent_mut(1).flags |= ENT_HIDDEN;
        let label = format!("fr_vs_action_{}", self.cur().vs_anim + 1);
        let f = &mut self.flash[FL_MOVE_A];
        f.play(1);
        f.goto_label(&label, true);
    }

    /// `Bracket_StartWin`: the winner's chip moves up.
    fn start_win(&mut self) {
        self.flags |= MOVE_B;
        self.tex[FL_MOVE_B][1] = self.chip(self.ent(0).chara);
        self.tex[FL_MOVE_B][4] = self.chip(self.ent(1).chara);
        self.scroll = -(view_x(self.cur().view) as f32);
        self.flash[FL_TREE].set_offset(self.scroll as i32, 0);
        self.ent_mut(0).flags |= ENT_HIDDEN;
        self.ent_mut(1).flags |= ENT_HIDDEN;
        let side = if self.win_side != 0 { 'R' } else { 'L' };
        let label = format!("fr_win_action_{}{side}", self.cur().win_anim + 1);
        let f = &mut self.flash[FL_MOVE_B];
        f.play(1);
        f.goto_label(&label, true);
    }

    /// `Bracket_UpdateImages`: one step of the loader of the two fighters' large pictures.
    fn update_images(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_REQUEST => {
                self.tex[FL_VS][6] = None;
                self.tex[FL_VS][5] = None;
                let chara = [self.ent(0).chara, self.ent(1).chara];
                self.loading = chara.map(|c| loader.portrait(FACE_FILE + c.max(0) as u32));
                self.load_state = LOAD_READ;
            }
            // File_UpdateRequests: the reads are done by the next call.
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[FL_VS][6] = self.loading[0];
                self.tex[FL_VS][5] = self.loading[1];
                self.flags |= IMAGES;
                self.load_state = 0;
            }
            _ => {}
        }
    }

    /// `Bracket_LoadImages`: the two large pictures at once.
    fn load_images(&mut self, loader: &mut dyn PictureLoader) {
        let chara = [self.ent(0).chara, self.ent(1).chara];
        self.tex[FL_VS][6] = loader.portrait(FACE_FILE + chara[0].max(0) as u32);
        self.tex[FL_VS][5] = loader.portrait(FACE_FILE + chara[1].max(0) as u32);
    }

    /// `Bracket_PickStage`: a random unlocked stage of a list (eight for the Yamcha Game, 23 for the others).
    fn pick_stage(yamcha: bool, save: &SaveData, random: &mut GameRandom) -> i32 {
        const YAMCHA_STAGES: [i32; 8] = [2, 3, 7, 8, 13, 16, 18, 34];
        const STAGES: [i32; 23] = [
            0, 1, 5, 9, 10, 11, 12, 14, 15, 17, 19, 20, 21, 22, 23, 24, 25, 26, 29, 30, 31, 32, 33,
        ];
        let table: &[i32] = if yamcha { &YAMCHA_STAGES } else { &STAGES };
        let list: Vec<i32> = table
            .iter()
            .copied()
            .filter(|&s| save.stage_unlocked(s))
            .collect();
        let n = random.range(list.len() as u32) as usize;
        list.get(n).copied().unwrap_or(0)
    }

    /// `Bracket_SetupBattle`: a player is battle side 0 on pad 0; a second player side 1 on pad 1 (split screen),
    /// a computer side 1 with the level and items of the table `[level][round]`. In the Cell Games the players'
    /// health carries over from their last battle plus 20, at most 100.
    fn setup_battle(&self, save: &SaveData, random: &mut GameRandom) -> BattleSetup {
        let cpu = self
            .data
            .cpu
            .get((self.session.level * ROUND_NUM + self.round).max(0) as usize)
            .copied()
            .unwrap_or_default();
        let cpu_items = cpu.items.map(|i| i.wrapping_add(1));
        let cell = self.tour() == CELL;
        let carried = |e: &TourEntrant| {
            if !cell {
                100.0
            } else if self.round != 0 {
                (e.health + 20.0).min(100.0)
            } else {
                100.0
            }
        };
        let (l, r) = (*self.ent(0), *self.ent(1));
        let (screen_mode, control, members) = if l.flags & ENT_PLAYER != 0 {
            if r.flags & ENT_PLAYER != 0 {
                (1, 0, [(l, l.items, carried(&l)), (r, r.items, carried(&r))])
            } else {
                (0, 2, [(l, l.items, carried(&l)), (r, cpu_items, 100.0)])
            }
        } else {
            (0, 2, [(r, r.items, carried(&r)), (l, cpu_items, 100.0)])
        };
        let (stage, announcer, bgm) = match self.tour() {
            WORLD => (if self.round == 4 { 0x1B } else { 4 }, 4, 0x12),
            BIG => {
                let stage = Self::pick_stage(false, save, random);
                (stage, 5, random.range(9) as i32 + 8)
            }
            CELL => (
                if self.round == 4 { 6 } else { 0x1C },
                6,
                random.range(9) as i32 + 8,
            ),
            OTHERWORLD => {
                let stage = Self::pick_stage(false, save, random);
                (stage, 2, random.range(9) as i32 + 8)
            }
            _ => {
                let stage = Self::pick_stage(true, save, random);
                (stage, 3, random.range(9) as i32 + 8)
            }
        };
        let side = |i: usize| {
            let (e, items, health) = members[i];
            SideSetup {
                control: if i == 0 { 0 } else { control },
                pad: i as i32,
                change_allowed: 1,
                switch_enabled: 1,
                chara: e.chara,
                costume: e.costume,
                cpu_level: cpu.cpu_level,
                items,
                members: Some(
                    [MemberStart {
                        cpu_level: cpu.cpu_level,
                        health,
                        variant: 0,
                    }; 5],
                ),
                ..SideSetup::default()
            }
        };
        let mut setup = BattleSetup {
            wide: false,
            screen_mode,
            mode: 4,
            bgm,
            time_limit: 0,
            announcer,
            stage,
            stage_change: 1,
            sides: [side(0), side(1)],
            pool: Vec::new(),
            tutorial: 0,
        };
        setup.finish();
        setup
    }

    /// `Bracket_DrawDragonBall`: a missing dragon ball, by a chance (percent) by tournament, place and level.
    fn draw_dragon_ball(&self, second: bool, save: &SaveData, random: &mut GameRandom) -> i32 {
        const CHANCE: [[[u32; 3]; 2]; 5] = [
            [[20, 30, 40], [5, 10, 15]],
            [[20, 30, 40], [5, 10, 15]],
            [[20, 30, 40], [5, 10, 15]],
            [[20, 30, 40], [5, 10, 15]],
            [[40, 50, 60], [15, 20, 25]],
        ];
        let owned = save.unlock_flags();
        if owned & 0x7F == 0x7F {
            return -1;
        }
        let chance = CHANCE[self.tour() as usize][usize::from(second)]
            [self.session.level.clamp(0, 2) as usize];
        if random.range(100) >= chance {
            return -1;
        }
        let list: Vec<i32> = (0..7).filter(|i| owned & (1 << i) == 0).collect();
        list[random.range(list.len() as u32) as usize]
    }

    /// `Bracket_GivePrizes`: the prizes of the tournament at its level (the runner-up only the second-place
    /// money), paid into the save and listed for the reward window; a dragon ball may come on top.
    fn give_prizes(&mut self, second: bool, save: &mut SaveData, random: &mut GameRandom) {
        self.reward.clear();
        let level = self.session.level.clamp(0, 2) as usize;
        let table = self
            .data
            .prizes
            .get(self.tour() as usize)
            .copied()
            .unwrap_or([[-1; 3]; PRIZE_NUM]);
        for (kind, row) in table.iter().enumerate() {
            let value = row[level];
            if value < 0 {
                continue;
            }
            if !second {
                match kind {
                    PRIZE_MONEY_WIN => {
                        self.reward.push((get_win::KIND_MONEY, value));
                        save.add_money(value);
                    }
                    PRIZE_ITEM => {
                        if save.item(value as usize) & 1 == 0 {
                            self.reward.push((get_win::KIND_ITEM, value));
                            save.add_item(value as usize);
                        }
                    }
                    PRIZE_CHARA_A | PRIZE_CHARA_B => {
                        if !save.chara_unlocked(value) {
                            self.reward.push((get_win::KIND_CHARA, value));
                            if let Some(w) = save.chara_bits.get_mut(value as usize / 64) {
                                *w |= 1 << (value % 64);
                            }
                        }
                    }
                    PRIZE_STAGE => {
                        if !save.stage_unlocked(value) {
                            self.reward.push((get_win::KIND_STAGE, value));
                            if value < 64 {
                                save.stage_bits |= 1 << value;
                            }
                        }
                    }
                    _ => {}
                }
            } else if kind == PRIZE_MONEY_SECOND {
                self.reward.push((get_win::KIND_MONEY, value));
                save.add_money(value);
            }
        }
        let ball = self.draw_dragon_ball(second, save, random);
        if ball >= 0 {
            self.reward.push((get_win::KIND_DRAGON_BALL, ball));
            save.set_unlock_flags(save.unlock_flags() | 1 << ball);
        }
        self.reward.truncate(REWARD_MAX);
    }

    fn say(&mut self, line: i32, sounds: &mut Vec<TitleSound>) {
        self.voice_line = line;
        anim::play_with_subtitle(&self.data.lips, VOICE_BASE, line, &mut self.lip, sounds);
        self.set_talker(line);
    }

    /// `Bracket_SetTalker`.
    fn set_talker(&mut self, line: i32) {
        let (pose, talker) = talker_for(line);
        if let Some(t) = talker {
            self.talker = t;
        }
        let (Some(pose), Some(sets)) = (pose, &self.data.layout.poses) else {
            return;
        };
        self.pose = pose;
        let g = &mut self.tex[FL_GUIDE];
        let at = if pose == 0 { [0, 2, 1] } else { [3, 5, 4] };
        let off = if pose == 0 { [3, 5, 4] } else { [0, 2, 1] };
        for k in 0..3 {
            g[at[k]] = sets[pose][k];
            g[off[k]] = None;
        }
    }

    /// The guide's speech before a battle: confirm skips it once the two pictures are loaded.
    fn skip_intro(&mut self, pressed: u32, sounds: &mut Vec<TitleSound>) {
        if pressed & PAD_CROSS != 0 && self.flags & IMAGES != 0 {
            self.seq = 0xB;
            sounds.push(TitleSound::Effect(1, 1));
        }
    }

    /// Confirm skips the speech of a new tournament.
    fn skip_opening(&mut self, pressed: u32, sounds: &mut Vec<TitleSound>) {
        if pressed & PAD_CROSS != 0 {
            self.seq = 0x39;
            sounds.push(TitleSound::Effect(1, 1));
        }
    }

    /// The banner of a result, the guide away and the tree sliding out.
    fn banner(&mut self, label: &str) {
        self.flash[FL_TITLE].goto_label(label, true);
        self.flash[FL_GUIDE].goto_label("fl_guide_out", true);
        self.msg.close();
        self.flags |= SCROLL_BACK;
        self.scroll_speed = (0x200 - self.scroll as i32) / 30;
        self.seq += 1;
    }

    /// The guide's closing line after the player won.
    fn victory_speech(&mut self, sounds: &mut Vec<TitleSound>) {
        self.flags &= !NO_TEXT;
        let line = match self.tour() {
            WORLD => 0x4C,
            BIG => 0x9E,
            CELL => 0x79,
            OTHERWORLD => 0xC9,
            _ => 0xF5,
        };
        self.say(line, sounds);
        self.seq += 1;
        self.flags |= VS_FRONT;
        self.flash[FL_GUIDE].goto_label("fl_guide_in", true);
        self.msg.open();
    }

    fn leave_over(&mut self) {
        self.seq = 0;
        self.flags |= DONE | LEAVING;
        self.timer = 0xF;
        self.result = 0;
    }

    /// `Bracket_UpdateSeq`: the guide's speech and the result sequences, one step a frame.
    #[allow(clippy::too_many_arguments)]
    fn update_seq(
        &mut self,
        pressed: u32,
        voice_stat: i32,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        mc: &mut McEnv,
        sounds: &mut Vec<TitleSound>,
    ) {
        let idle = voice_stat == VOICE_PLAYEND;
        let tour = self.tour();
        match self.seq {
            0 => {}
            // Before a battle, in front of the versus panel.
            1 => {
                self.flags |= NO_TEXT;
                if self.session.greeted {
                    self.seq = 9;
                    return;
                }
                let base = match tour {
                    WORLD => 0x32,
                    BIG => 0x7E,
                    CELL => 0x5F,
                    OTHERWORLD => 0xAF,
                    _ => 0xDB,
                };
                self.say(self.round * 2 + base, sounds);
                self.seq += 1;
                self.skip_intro(pressed, sounds);
            }
            2 | 6 | 8 | 0xA => {
                if idle {
                    self.seq += 1;
                } else {
                    self.skip_intro(pressed, sounds);
                }
            }
            3 | 5 => {
                self.say(self.voice_line + 1, sounds);
                self.seq += 1;
                self.skip_intro(pressed, sounds);
            }
            4 => {
                if idle {
                    self.seq += if self.round == 4 { 1 } else { 5 };
                } else {
                    self.skip_intro(pressed, sounds);
                }
            }
            7 => {
                let line = match tour {
                    WORLD => 0x3D,
                    BIG => 0x89,
                    CELL => 0x6A,
                    OTHERWORLD => 0xBA,
                    _ => 0xE6,
                };
                self.say(line, sounds);
                self.seq += 1;
                self.skip_intro(pressed, sounds);
            }
            9 => {
                // A random cheer.
                let line = match tour {
                    WORLD => random.range(14) as i32 + 0x3E,
                    BIG => random.range(7) as i32 + 0x8A,
                    CELL => random.range(14) as i32 + 0x6B,
                    OTHERWORLD => random.range(14) as i32 + 0xBB,
                    _ => random.range(14) as i32 + 0xE7,
                };
                self.say(line, sounds);
                self.seq += 1;
            }
            0xB => {
                // Off to the battle.
                self.timer = 0xC;
                self.voice_line = -1;
                self.flags &= !NO_TEXT;
                self.flags |= LEAVING;
                self.seq = 0;
                self.session.greeted = true;
            }
            // A new tournament opens, in front of the title.
            0x33 | 0x34 => {
                if self.seq == 0x33 {
                    self.seq += 1;
                }
                if self.flash[FL_TITLE].flags() & FLASH_PAD != 0 {
                    self.seq += 1;
                }
            }
            0x35 => {
                self.flags &= !NO_TEXT;
                let (line, next) = match tour {
                    WORLD => (0x24, 0x38),
                    BIG => (0x7D, 0x38),
                    CELL => (0x50, 0x36),
                    OTHERWORLD => (0xA1, 0x38),
                    _ => (0xCD, 0x38),
                };
                self.seq = next;
                self.say(line, sounds);
                self.skip_opening(pressed, sounds);
            }
            0x36 | 0x38 => {
                if idle {
                    self.seq += 1;
                }
                self.skip_opening(pressed, sounds);
            }
            0x37 => {
                self.say(self.voice_line + 1, sounds);
                self.seq += 1;
                self.skip_opening(pressed, sounds);
            }
            0x39 => {
                self.flash[FL_TITLE].goto_label("fl_direct_out", true);
                self.flash[FL_TREE].play(1);
                self.seq = 0x65;
            }
            // The round is announced over the tree.
            0x65 => {
                self.flags &= !NO_TEXT;
                self.flags |= RESULT0;
                let line = if self.round == 0 {
                    match tour {
                        WORLD => 0x25,
                        BIG => 0x91,
                        CELL => 0x52,
                        OTHERWORLD => 0xA2,
                        _ => 0xCE,
                    }
                } else {
                    let base = match tour {
                        WORLD => 0x23,
                        BIG => 0x8F,
                        CELL => 0x50,
                        OTHERWORLD => 0xA0,
                        _ => 0xCC,
                    };
                    self.round * 3 + base
                };
                self.say(line, sounds);
                self.seq += 1;
                if pressed & PAD_CROSS != 0 {
                    self.seq = 0x67;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            0x66 | 0x68 => {
                if idle {
                    self.seq += 1;
                } else if pressed & PAD_CROSS != 0 {
                    self.seq += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            0x67 => {
                if self.round != 0 {
                    // One of two follow-up lines: libc rand(), not the shared generator.
                    let line = self.voice_line + (libc.rand() & 1) + 1;
                    self.say(line, sounds);
                    self.seq += 1;
                } else {
                    self.seq = 0x69;
                }
            }
            0x69 => {
                self.seq = 0;
                self.flags &= !RESULT0;
                sounds.push(TitleSound::VoiceStop);
                self.lip = LipSync::default();
                self.voice_line = -1;
            }
            // Back from a battle: the versus panel shows the outcome until confirm.
            0x97 => {
                self.seq += 1;
                self.flags |= NO_TEXT;
            }
            0x98 | 0xFE | 0x130 | 0x161 | 0xCB => {
                if pressed & PAD_CROSS != 0 {
                    self.seq += 1;
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            0x99 => {
                self.voice_line = -1;
                self.flags &= !NO_TEXT;
                self.flags &= !RESULT;
                self.seq = 0;
                self.flash[FL_VS].goto_label("fl_out", true);
                self.flash[FL_TREE].play(1);
                self.flash[FL_GUIDE].play(1);
                self.flash[FL_GUIDE].goto_label("fl_guide_in", true);
                self.msg.open();
            }
            // The player won the open tournament: banner, speech, prizes, save.
            0xFB | 0x12D => self.banner("fl_yusyo_in"),
            0xFC | 0x100 | 0x12E | 0x160 | 0x163 | 0xCA => {
                if self.flash[FL_TITLE].flags() & FLASH_END != 0 {
                    self.seq += 1;
                }
            }
            0xFD | 0x12F => self.victory_speech(sounds),
            0xFF => {
                self.flash[FL_GUIDE].goto_label("fl_guide_out", true);
                self.msg.close();
                self.flash[FL_TITLE].goto_label("fl_yusyo_out", true);
                self.give_prizes(false, mc.save, random);
                self.reward_idx = 0;
                self.seq += 1;
            }
            0x101 | 0x164 => {
                let (kind, value) = self.reward.get(self.reward_idx).copied().unwrap_or((0, 0));
                self.get_win.setup(kind, value);
                if self.reward_idx != 0 {
                    self.get_win.next(sounds);
                } else {
                    self.get_win.open(sounds);
                }
                self.reward_idx += 1;
                self.seq += 1;
            }
            0x102 | 0x105 | 0x165 | 0x168 => {
                if self.get_win.is_anim_done() {
                    self.seq += 1;
                }
            }
            0x103 | 0x166 => {
                if pressed & PAD_CROSS != 0 {
                    if self.reward_idx >= self.reward.len() {
                        self.seq += 1;
                    } else {
                        self.seq -= 2;
                    }
                    sounds.push(TitleSound::Effect(1, 1));
                }
            }
            0x104 | 0x167 => {
                self.seq += 1;
                self.get_win.close();
            }
            0x106 | 0x169 => {
                self.seq = 0;
                self.mcflow.start(mcflow::MODE_SAVE, mc);
            }
            // The player won a free tournament: banner and speech only.
            0x131 => self.leave_over(),
            // The player lost the final of the open tournament: banner, the runner-up's prizes, save.
            0x15F | 0xC9 => self.banner("fl_haiboku_in"),
            0x162 => {
                self.flash[FL_TITLE].goto_label("fl_jyunyusyo_in", true);
                self.give_prizes(true, mc.save, random);
                self.reward_idx = 0;
                self.seq += 1;
            }
            // The player lost.
            0xCC => self.leave_over(),
            _ => sounds.push(TitleSound::VoiceFadeOutStep),
        }
    }

    /// `Bracket_Update`: the movies advance; the jingle hands over to the music; the two chips meet, the winner
    /// moves up and the next step is chosen.
    fn update(&mut self, stream_stat: i32, sounds: &mut Vec<TitleSound>) {
        for f in &mut self.flash {
            f.advance();
        }
        if self.flags & FANFARE != 0 && stream_stat == VOICE_PLAYEND {
            sounds.push(TitleSound::Bgm(self.bgm()));
            self.flags &= !FANFARE;
        }
        if self.flags & SCROLL_BACK != 0 {
            self.scroll += self.scroll_speed as f32;
            if self.scroll as i32 >= 0x200 {
                self.scroll = 512.0;
                self.flags ^= SCROLL_BACK;
            }
        }
        if self.flags & BUSY != 0 {
            return;
        }
        let tree = self.flash[FL_TREE].flags();
        if self.flags & (INPUT | MOVE_B) == 0 && tree & FLASH_PLAYING != 0 && tree & FLASH_END != 0
        {
            if self.session.running {
                self.start_win();
            } else {
                self.flags |= INPUT;
            }
        }
        if self.flags & MOVE_A != 0 && self.flash[FL_MOVE_A].flags() & FLASH_PLAYING != 0 {
            if self.flash[FL_MOVE_A].se() & 1 != 0 {
                sounds.push(TitleSound::Effect(2, 0x3A));
            }
            if self.flash[FL_MOVE_A].flags() & FLASH_END != 0 {
                self.flags ^= MOVE_A;
                self.seq = 1;
                self.flash[FL_VS].reset();
                self.flash[FL_VS].play(1);
                self.flash[FL_GUIDE].goto_label("fl_guide_out", true);
                self.msg.close();
                self.flags |= SCROLL_BACK;
                self.scroll_speed = (0x200 - self.scroll as i32) / 30;
            }
        }
        if self.flags & MOVE_B != 0 && self.flash[FL_MOVE_B].flags() & FLASH_END != 0 {
            self.flags ^= MOVE_B;
            self.flags |= INPUT;
            let win = self.win_side;
            self.ent_mut(win ^ 1).flags |= ENT_LOST;
            self.ent_mut(0).flags &= !ENT_HIDDEN;
            self.ent_mut(1).flags &= !ENT_HIDDEN;
            self.flash[FL_MOVE_B].set_flag(4, true);
            let m = self.cur();
            self.ent_mut(win).pos = i32::from(m.pos);
            if self.flags & PLAYER_OUT != 0 {
                self.flags |= BUSY;
                self.flags ^= INPUT;
                if self.session.entry == 0 && m.flags & MATCH_FINAL != 0 {
                    self.flags |= RUNNER_UP;
                    self.seq = 0x15F;
                } else {
                    self.seq = 0xC9;
                }
            } else {
                if m.flags & MATCH_FINAL != 0 {
                    if self.ent(win).flags & ENT_PLAYER != 0 {
                        self.flags ^= INPUT;
                        self.seq = if self.session.entry == 0 { 0xFB } else { 0x12D };
                        self.flags |= BUSY;
                        return;
                    }
                } else {
                    let next = usize::from(m.next).min(MATCH_MAX - 1);
                    if m.flags & MATCH_TO_LEFT != 0 {
                        self.matches[next].ent[0] = m.ent[win];
                    } else if m.flags & MATCH_TO_RIGHT != 0 {
                        self.matches[next].ent[1] = m.ent[win];
                    }
                }
                self.next_match();
            }
        }
    }

    /// `Bracket_SetupDraw`: the clips that follow the bracket's state.
    fn setup_draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &Font,
        texts: &mut Vec<TextDraw>,
    ) {
        let uv = |u0: i32, v0: i32, u1: i32, v1: i32| Uv {
            u0,
            v0,
            u1,
            v1,
            tex: 0,
        };
        // The title plate: the round's name; the result banners.
        {
            let f = &mut self.flash[FL_TITLE];
            let r = f.find(Some("mc_menu_title_plate"), "mc_menu_title_text02");
            f.clip_set_uv(r, uv(0, self.round * 0x20, 0x100, self.round * 0x20 + 0x20));
            let r = f.find(None, "mc_victory_lose_result_text01");
            f.clip_set_uv(r, uv(0, 0x80, 0x200, 0x100));
            let v0 = if self.flags & RUNNER_UP != 0 { 0x40 } else { 0 };
            let r = f.find(Some("mc_victory_plate"), "mc_menu_victory_text");
            f.clip_set_uv(r, uv(0, v0, 0x100, v0 + 0x40));
        }
        // The guide: blink and mouth.
        let tour = self.tour();
        {
            let f = &mut self.flash[FL_GUIDE];
            let lip = &mut self.lip;
            match tour {
                WORLD | OTHERWORLD => {
                    let (clip, mouth) = if tour == WORLD {
                        ("mc_guide_tenkaichi", "mc_guide_tenkaichi_mouth")
                    } else {
                        ("mc_guide_anoyo", "mc_guide_anoyo_mouth")
                    };
                    let r = f.find(Some(clip), mouth);
                    anim::talk(f, r, &mut self.talk[0], 0, lip, voice_stat, random);
                }
                BIG | CELL => {
                    let n = self.pose + 1;
                    let (clip, eye, mouth) = if tour == BIG {
                        (
                            format!("mc_guide_satan_s_0{n}"),
                            format!("mc_guide_satan_s_0{n}_eye"),
                            format!("mc_guide_satan_s_0{n}_mouth"),
                        )
                    } else {
                        (
                            format!("mc_guide_seru_0{n}"),
                            format!("mc_guide_ceru_0{n}_eye"),
                            format!("mc_guide_ceru_0{n}_mouth"),
                        )
                    };
                    let r = f.find(Some(&clip), &eye);
                    anim::blink(f, r, &mut self.blink[0], 0, random);
                    let r = f.find(Some(&clip), &mouth);
                    anim::talk(f, r, &mut self.talk[0], 0, lip, voice_stat, random);
                }
                _ => {
                    for i in 0..2 {
                        let clip = if i != 0 {
                            "mc_guide_puaru"
                        } else {
                            "mc_guide_yamucha"
                        };
                        let (eye, mouth) = match (i, self.pose) {
                            (0, _) => ("mc_guide_yamucha_eye", "mc_guide_yamucha_mouth"),
                            (_, 0) => ("mc_guide_puaru_eye01", "mc_guide_puaru_mouth01"),
                            _ => ("guide_puaru_eye02", "mc_guide_puaru_mouth02"),
                        };
                        let r = f.find(Some(clip), eye);
                        anim::blink(f, r, &mut self.blink[i], 0, random);
                        let r = f.find(Some(clip), mouth);
                        if self.talker == i {
                            anim::talk(f, r, &mut self.talk[i], 0, lip, voice_stat, random);
                        } else {
                            anim::show_next(f, r, 0);
                        }
                    }
                }
            }
        }
        // The tree: scroll, arrows, the seventeen chips.
        let scroll = self.scroll as i32;
        {
            let input = self.flags & INPUT != 0;
            let f = &mut self.flash[FL_TREE];
            f.set_offset(scroll, 0);
            let r = f.find(None, "mc_scroll_yazirusi");
            f.clip_set_offset(r, -scroll, 0);
            for i in 0..2 {
                let name = if i != 0 {
                    "mc_yajirusi_l"
                } else {
                    "mc_yajirusi_r"
                };
                let r = f.find(Some("mc_scroll_yazirusi"), name);
                f.clip_set_uv(r, uv(i * 0x40, 0, i * 0x40 + 0x40, 0x40));
                let shown = input && !((scroll <= -0x200 && i == 0) || (scroll >= 0 && i != 0));
                f.clip_set_flags(r, PROP_VISIBLE, shown);
            }
            for (i, e) in self.ents.iter().enumerate() {
                let r = f.find(None, &format!("mc_tournament_frame_{i}"));
                if e.flags & ENT_PLAYER != 0 {
                    let (u, v) = ((e.player % 4) * 0x40, (e.player / 4) * 0x40);
                    f.clip_set_tex(r, 1);
                    f.clip_set_uv(r, uv(u, v, u + 0x40, v + 0x40));
                }
                let r = f.find(None, &format!("mc_tournament_chara{i}"));
                f.clip_set_offset(r, pos_x(e.pos), pos_y(e.pos));
                if e.flags & ENT_HIDDEN != 0 {
                    f.clip_set_flags(r, PROP_VISIBLE, false);
                } else {
                    f.clip_set_flags(r, PROP_VISIBLE, true);
                    if e.flags & ENT_LOST != 0 {
                        f.clip_set_color(r, 0.6);
                    }
                }
            }
        }
        // The two chips of the current match, moving.
        let pair = [*self.ent(0), *self.ent(1)];
        for movie in [FL_MOVE_A, FL_MOVE_B] {
            let f = &mut self.flash[movie];
            if f.flags() & FLASH_PLAYING == 0 {
                continue;
            }
            for (i, e) in pair.iter().enumerate() {
                if e.flags & ENT_PLAYER != 0 {
                    let (u, v) = ((e.player % 4) * 0x40, (e.player / 4) * 0x40);
                    let r = f.find(None, &format!("mc_tournament_frame_{i}"));
                    f.clip_set_tex(r, 1);
                    f.clip_set_uv(r, uv(u, v, u + 0x40, v + 0x40));
                }
            }
            f.set_offset(pos_x(pair[0].pos) + scroll, pos_y(pair[0].pos));
        }
        // The versus panel: names, forms, player numbers, win / lose plates.
        if self.flash[FL_VS].flags() & FLASH_PLAYING != 0 {
            let f = &mut self.flash[FL_VS];
            for (i, e) in pair.iter().enumerate() {
                let line = if e.flags & ENT_REC != 0 {
                    0xA3
                } else {
                    // Two pairs of characters share a name line.
                    match e.chara {
                        0x16 | 0x17 => 0xA7,
                        0x3A | 0x3B => 0xA5,
                        c => c,
                    }
                };
                let side = if i != 0 { 'r' } else { 'l' };
                let r = f.find(None, &format!("mc_name_text_{side}"));
                texts.extend(TextBox::preset(i as u32 + 1).attach_line(
                    f,
                    r,
                    0x400 + i as u32,
                    &self.data.names,
                    line,
                    font,
                ));
                let r = f.find(None, &format!("mc_form_text_{side}"));
                texts.extend(TextBox::preset(i as u32 + 3).attach_line(
                    f,
                    r,
                    0x402 + i as u32,
                    &self.data.forms,
                    line,
                    font,
                ));
                let number = if e.flags & ENT_PLAYER != 0 {
                    let (u, v) = ((e.player % 5) * 0x60, (e.player / 5) * 0x40);
                    uv(u, v, u + 0x60, v + 0x40)
                } else {
                    uv(0x120, 0x40, 0x180, 0x80)
                };
                let r = f.find(None, &format!("mc_vs_number_{side}"));
                f.clip_set_uv(r, number);
                let won = self.win_side == i;
                let plate = if self.round == 4 {
                    if won { 3 } else { 2 }
                } else {
                    i32::from(won)
                };
                let r = f.find(None, &format!("mc_victory_or_defeat_{side}"));
                f.clip_set_uv(r, uv(0, plate * 0x40, 0x100, plate * 0x40 + 0x40));
            }
        }
    }

    fn draw_flash(
        &self,
        i: usize,
        quads: &mut Vec<ScreenQuad>,
        font: &mut Font,
        texts: &[TextDraw],
    ) {
        let tex = &self.tex[i];
        let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
        let (fq, overs) = self.flash[i].draw_with_overs(&has_texture);
        quads.extend(screen_quads(&fq, tex));
        if !texts.is_empty() {
            draw_texts(texts, &overs, font);
            font.flush(quads);
        }
    }

    /// `Bracket_Draw`: the backdrop, the tree, the moving chips, the guide with its window, the panels and the
    /// reward window (the panels under the guide while a result is told).
    fn draw(
        &mut self,
        voice_stat: i32,
        random: &mut GameRandom,
        font: &mut Font,
        quads: &mut Vec<ScreenQuad>,
    ) {
        self.bg.draw(quads);
        let mut texts = Vec::new();
        self.setup_draw(voice_stat, random, font, &mut texts);
        for i in [FL_TREE, FL_MOVE_A, FL_MOVE_B] {
            self.draw_flash(i, quads, font, &[]);
        }
        let line = if self.flags & NO_TEXT != 0 {
            -1
        } else {
            self.voice_line
        };
        if self.flags & VS_FRONT != 0 {
            self.draw_flash(FL_VS, quads, font, &texts);
            self.draw_flash(FL_TITLE, quads, font, &[]);
            self.msg.draw(line, font, quads);
            self.draw_flash(FL_GUIDE, quads, font, &[]);
        } else {
            self.msg.draw(line, font, quads);
            self.draw_flash(FL_GUIDE, quads, font, &[]);
            self.draw_flash(FL_VS, quads, font, &texts);
            self.draw_flash(FL_TITLE, quads, font, &[]);
        }
        self.get_win.draw(font, quads);
    }

    /// `Bracket_Input`: left / right scroll the tree while it may be looked at; confirm starts the next match.
    fn input(&mut self, held: u32, pressed: u32, sounds: &mut Vec<TitleSound>) {
        if self.flags & INPUT == 0 {
            return;
        }
        if held & PAD_LEFT != 0 {
            if self.scroll < 0.0 {
                self.scroll = (self.scroll + 3.0).min(0.0);
            }
        } else if held & PAD_RIGHT != 0 {
            if self.scroll > -512.0 {
                self.scroll = (self.scroll - 3.0).max(-512.0);
            }
        } else if pressed & PAD_CROSS != 0 {
            self.start_vs();
            self.flags ^= INPUT;
            sounds.push(TitleSound::Effect(1, 1));
        }
    }

    /// One iteration of `Bracket_Run`'s loop. `stream_stat` is `StreamSe_GetStat(0)`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        stream_stat: i32,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
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
        self.update_images(loader);
        let repeat = self.pad.update(held, pressed);
        self.fade.update();
        self.update_seq(pressed, voice_stat, random, libc, mc, &mut frame.sounds);
        self.update(stream_stat, &mut frame.sounds);
        self.draw(voice_stat, random, font, &mut frame.quads);
        let mut effects = Vec::new();
        let mut io = McIo {
            dialog: &mut self.dialog,
            pad: Pad { pressed, repeat },
            sounds: &mut effects,
        };
        let (state, events) = self.mcflow.update(mc, &mut io, font, &mut frame.quads);
        self.mc_state = state;
        // Bracket_OnSaveDone is the flow's done callbacks 0 and 1: the tournament is over.
        if events
            .iter()
            .any(|e| matches!(e, mcflow::Event::Done(Done::Done | Done::NotDone)))
        {
            self.flags |= DONE | LEAVING;
            self.timer = 0xF;
            self.result = 0;
        }
        frame.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done() && !self.started {
            self.started = true;
            self.seq = if self.session.running { 0x97 } else { 0x33 };
        }
        if self.fade.is_fading_out() {
            frame.sounds.push(TitleSound::FadeOutStep);
            return frame;
        }
        if self.fade.is_out_done() {
            if self.load_state == 0 {
                self.leave(mc.save, random);
            }
            return frame;
        }
        if self.flags & LEAVING != 0 {
            self.timer -= 1;
            if self.timer == -1 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.mc_state == 0 {
            self.input(held, pressed, &mut frame.sounds);
        }
        frame
    }

    /// The end of `Bracket_Run`: on to the battle with the tournament written back, or the tournament cleared.
    fn leave(&mut self, save: &SaveData, random: &mut GameRandom) {
        if self.result != 0 {
            self.session.running = true;
            self.session.round = self.round;
            self.session.match_no = self.match_no as i32;
            self.session.entrants = self.ents;
            self.session.matches = self.matches;
            let setup = self.setup_battle(save, random);
            self.done = Some(BracketResult::Battle(Box::new(setup)));
        } else {
            self.session.round = 0;
            self.session.match_no = 0;
            self.session.entrants = Default::default();
            self.session.matches = Default::default();
            self.session.running = false;
            self.session.greeted = false;
            self.done = Some(BracketResult::Over);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memcard::{Libmc, McCard};

    struct Pictures;

    impl PictureLoader for Pictures {
        fn portrait(&mut self, file_id: u32) -> Option<usize> {
            Some(file_id as usize)
        }

        fn picture(&mut self, _: u32) -> Option<usize> {
            None
        }
    }

    #[test]
    fn the_tree_positions_follow_the_matches() {
        let m = init_matches();
        assert_eq!(m[0].ent, [0, 1]);
        assert_eq!(m[7].ent, [14, 15]);
        assert_eq!(m[15].ent[0], 16);
        // Each winner's position lies one round up, between the two it came from.
        for (i, mt) in m.iter().enumerate().take(8) {
            let x = pos_x(i32::from(mt.pos));
            assert!(pos_x(2 * i as i32) < x && x < pos_x(2 * i as i32 + 1));
            assert!(pos_y(i32::from(mt.pos)) < pos_y(0));
        }
        assert_eq!(
            m.iter().filter(|m| m.flags & MATCH_ROUND_END != 0).count(),
            5
        );
    }

    /// Runs the real bracket of the World Tournament from the ISO in `rom/`, when it is there: a new tournament
    /// with one entrant draws sixteen others and the boss, announces the first round, and confirm sends the
    /// player's match to the battle; back from it with a win, the CPU matches of the round are decided, the
    /// winner's chip moves up, and the player's next match comes on.
    #[test]
    fn real_bracket_runs_a_round() {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = bt3_data::rom::for_tests();
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let mut read = |id: u32| {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .unwrap();
            archive.read_entry(&mut image, location.index).unwrap()
        };
        let chr = ChrTable::parse(&read(4), 0xA1);
        let res = bpe::decode(&read(PACK_FILE_FIRST)).unwrap();
        let assets = BracketAssets::parse(&res, WORLD).unwrap();
        let bg_res = bpe::decode(&read(crate::tour_bg::FILE_FIRST)).unwrap();
        let bg_assets = crate::tour_bg::TourBgAssets::parse(&bg_res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        data.layout = BracketLayout::new(WORLD, vec![None; 16], |k| firsts[&k]);
        assert_eq!(data.cpu.len(), 15);
        assert_eq!(data.prizes[0][PRIZE_MONEY_WIN], [3000, 5000, 8000]);
        let bg_data = crate::tour_bg::TourBgData {
            fod: bg_assets.fod,
            background: 0,
            first: 10,
        };
        let make_bg =
            || crate::tour_bg::TourBg::new(&bg_data, WORLD, Some(data.layout.bg_extra)).unwrap();
        let mut session = TourSession {
            entry: 1,
            entry_num: 1,
            ..TourSession::default()
        };
        session.entrants[0] = TourEntrant {
            flags: ENT_PLAYER,
            chara: 0,
            ..TourEntrant::default()
        };
        let mut save = SaveData::default();
        let mut random = GameRandom::from_key(&[3, 5]);
        let mut libc = LibcRandom::new(1);
        let mut sounds = Vec::new();
        let mut screen = BracketScreen::new(
            &data,
            make_bg(),
            &session,
            BattleOutcome::default(),
            &chr,
            &save,
            None,
            &mut Pictures,
            &mut random,
            &mut sounds,
        )
        .unwrap();
        assert!(sounds.contains(&TitleSound::StreamSe(SE_FANFARE)));
        // Seventeen different entrants, the boss last; the player left of its pair.
        let charas: std::collections::BTreeSet<i32> = screen.ents.iter().map(|e| e.chara).collect();
        assert_eq!(charas.len(), ENTRANT_MAX);
        assert!(
            screen.ents[16].flags & ENT_BOSS != 0 && [0x38, 0x42].contains(&screen.ents[16].chara)
        );
        let p = screen
            .ents
            .iter()
            .position(|e| e.flags & ENT_PLAYER != 0)
            .unwrap();
        assert_eq!(p % 2, 0);
        assert_eq!(screen.match_no, p / 2);
        let mut card = McCard::new(Libmc::new(None));
        let mut progress = crate::mcflow::Progress::default();
        let items = chr.items.clone();
        let mut font = Font::new(Vec::new(), 20);
        let mut run = |screen: &mut BracketScreen,
                       save: &mut SaveData,
                       random: &mut GameRandom,
                       bits: u32,
                       frames: usize| {
            for i in 0..frames {
                let b = if i == 0 { bits } else { 0 };
                let mut mc = McEnv {
                    card: &mut card,
                    save,
                    progress: &mut progress,
                    items: &items,
                };
                screen.run_frame(
                    b,
                    b,
                    VOICE_PLAYEND,
                    VOICE_PLAYEND,
                    &mut Pictures,
                    random,
                    &mut libc,
                    &mut font,
                    &mut mc,
                );
                if screen.result().is_some() {
                    break;
                }
            }
        };
        // The opening, the round's announcement, then the tree takes input.
        for _ in 0..60 {
            if screen.flags & INPUT != 0 && screen.seq == 0 {
                break;
            }
            run(&mut screen, &mut save, &mut random, PAD_CROSS, 30);
        }
        assert!(screen.flags & INPUT != 0, "seq {:#x}", screen.seq);
        assert!(screen.flags & FANFARE == 0);
        // Confirm: the chips meet, the pictures load, the speech, off to the battle.
        run(&mut screen, &mut save, &mut random, PAD_CROSS, 1);
        assert!(screen.flags & MOVE_A != 0);
        for _ in 0..60 {
            if screen.result().is_some() {
                break;
            }
            run(&mut screen, &mut save, &mut random, PAD_CROSS, 30);
        }
        let Some(BracketResult::Battle(setup)) = screen.result().cloned() else {
            panic!("no battle: seq {:#x} flags {:#x}", screen.seq, screen.flags);
        };
        assert_eq!(setup.mode, 4);
        assert_eq!(setup.stage, 4);
        assert_eq!(setup.sides[0].chara, 0);
        assert_eq!(setup.sides[1].control, 2);
        assert_eq!(setup.sides[1].cpu_level, data.cpu[0].cpu_level);
        let session = screen.session();
        assert!(session.running && session.greeted);
        let opponent = session.matches[session.match_no as usize].ent[1];
        // Back from a won battle.
        let mut sounds = Vec::new();
        let mut screen = BracketScreen::new(
            &data,
            make_bg(),
            &session,
            BattleOutcome {
                winner: 1,
                health: [75.0, 0.0],
            },
            &chr,
            &save,
            None,
            &mut Pictures,
            &mut random,
            &mut sounds,
        )
        .unwrap();
        assert!(sounds.contains(&TitleSound::Bgm(BGM_WORLD)));
        assert!(screen.flags & RESULT != 0 && screen.flags & PLAYER_OUT == 0);
        assert_eq!(screen.ents[p].health, 75.0);
        // The round's other matches are decided.
        assert!((0..8).all(|m| screen.matches[m].flags & MATCH_DONE != 0));
        for _ in 0..60 {
            if screen.flags & INPUT != 0 && screen.seq == 0 {
                break;
            }
            run(&mut screen, &mut save, &mut random, PAD_CROSS, 30);
        }
        assert!(screen.flags & INPUT != 0, "seq {:#x}", screen.seq);
        assert!(screen.ents[usize::from(opponent)].flags & ENT_LOST != 0);
        assert_eq!(screen.ents[p].pos, 16 + (p / 2) as i32);
        assert_eq!(screen.match_no, 8 + p / 4);
        assert_eq!(screen.round, 1);
        assert!(screen.has_player(screen.match_no));
    }
}
