//! Sim Dragon's board (`SimDay_Run(0)`, src/menu/sim_day.c, progress mode 22, `Ub_Main`): a run is seven rounds
//! of ten turns. On the first nine turns of a round the player picks a button of the board (a training, a random
//! event, a rest, a full heal that then waits thirty turns) and the turn's event script plays out on the board's
//! movies (src/menu/sim_event*.c, here `sim_event`); the stat changes it queues are counted into the run's attack,
//! defence, health and points, the points raise the level that caps attack and defence. The tenth turn sets up the
//! round's fight from the pack's round and opponent tables, shows the two fighters and leaves for the battle.

use std::error::Error;

use bt3_data::gs::RgbaTexture;
use bt3_data::pack_section;

use crate::anim::{self, LipSync, VOICE_PLAYEND};
use crate::char_select::{BattleSetup, MemberStart, PictureLoader, SideSetup};
use crate::dialog::{Command, Dialog, DialogAssets, Pad};
use crate::draw::ScreenQuad;
use crate::flash::{FLASH_PAD, Flash, PROP_MASK, PROP_MASK_SET, PROP_VISIBLE, Uv, screen_quads};
use crate::font::Font;
use crate::logo::{ColorFade, picture_quads};
use crate::menu_res::{FileKey, ImageTexture, decode_files, decode_picture, image_indices, tex};
use crate::rng::{GameRandom, LibcRandom};
use crate::save::SaveData;
use crate::sim_event::{EventPad, SimGames};
use crate::sim_top::SimRun;
use crate::title::{
    PAD_CROSS, PAD_DOWN, PAD_START, PAD_TRIANGLE, PAD_UP, PadRepeat, TitleFrame, TitleSound,
};
use crate::ub::VOICE_BASE;
use crate::windows::{MsgWin, TextBox, TextDraw, draw_texts};

/// `QPROG->baseFile + 0x1A`: the board's own file (not a section of the group's archive).
pub const FILE_ID: u32 = 0x1C1 + 0x1A;
const FADE_FRAMES: i32 = 0x14;
/// `SIMDAY_BGM_FIRST` and `gSimBgm`: the music of each round.
const BGM_FIRST: u32 = 0x10B16;
const ROUND_BGM: [u32; 8] = [4, 1, 2, 0, 6, 4, 8, 0];
/// `SIMDAY_FACE_FILE`: + character id, a fighter's large picture.
const FACE_FILE: u32 = 0x2F9;
const DIALOG_SECTION: usize = 33;
/// `SIMDAY_RANDOM`, `SIMDAY_NONE`.
pub(crate) const RANDOM: i32 = 998;
pub(crate) const NONE: i32 = 999;
const EVENT_ROWS: usize = 32;
pub(crate) const TURNS: i32 = 10;
pub(crate) const LEVEL_MAX: i32 = 6;
const ITEM_NUM: usize = 3;
/// `SIM_LAST_TURN`.
const LAST_TURN: i32 = 0x45;
const ANDROID_16: i32 = 0x66;

// Stats.
pub(crate) const ATK: usize = 0;
pub(crate) const DEF: usize = 1;
pub(crate) const HP: usize = 2;
pub(crate) const POINT: usize = 3;

// SIMDAY_FL_*.
pub(crate) const FL_BOARD: usize = 0;
pub(crate) const FL_EVENT: usize = 1;
pub(crate) const FL_CARD: usize = 2;
pub(crate) const FL_POPO: usize = 3;
pub(crate) const FL_SHOT: usize = 4;
const FL_MENU: usize = 5;
const FL_ROUND: usize = 6;

// SIMDAY_*.
const LEAVING: u32 = 2;
const STARTED: u32 = 4;
const GREETED: u32 = 8;
const FACES_READY: u32 = 0x10;
const FACE_ON: u32 = 0x20;
const BUSY: u32 = 0x40;
pub(crate) const WAIT_KEY: u32 = 0x80;
const SAME_TURN: u32 = 0x100;
const SKIP_TURNS: u32 = 0x200;
pub(crate) const LEVEL_UP: u32 = 0x400;
/// `SIMEV_FLAG800` / `SIMEV_FLAG1000`: the next training comes out as outcome 0 / 5.
pub(crate) const TRAIN_WORST: u32 = 0x800;
pub(crate) const TRAIN_BEST: u32 = 0x1000;
const WINDOW: u32 = 0x2000;
// Flash.flags: the movie hit a trigger "end" action this tick.
const FLASH_END: u32 = 8;

// SIMDAY_LOAD_*.
const LOAD_IDLE: i32 = 0;
const LOAD_REQUEST: i32 = 1;
const LOAD_READ: i32 = 2;
const LOAD_UNPACK: i32 = 3;

// SIMDAY_ST_*.
const ST_BOARD: usize = 0;
const ST_TRAIN: usize = 1;
const ST_SELECT: usize = 2;
pub(crate) const ST_SCRIPT: usize = 3;
const ST_WAIT: usize = 4;
const ST_VERSUS: usize = 5;
const ST_WINDOW: usize = 6;
const ST_WINDOW_ITEMS: usize = 7;
const ST_CONFIRM: usize = 8;
const ST_BACK: usize = 9;
const ST_ROUND: usize = 10;
const ST_ROUND_WAIT: usize = 11;
const ST_QUIT: usize = 12;

/// One rank of a training (`SimTrain`): the card game's cards (`param`), the rock game's rocks and speed, the shell
/// game's swaps and speed; what a win pays and what a loss costs; the turn the rank starts on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Train {
    pub param: i32,
    /// The card game: seconds the cards lie face up; the other two: speed.
    pub speed: i32,
    pub points: i32,
    pub gain: i32,
    pub loss: i32,
    pub turn: i32,
}

const fn train(param: i32, speed: i32, points: i32, turn: i32) -> Train {
    Train {
        param,
        speed,
        points,
        gain: 5,
        loss: -5,
        turn,
    }
}

/// `gSimTrain0`..`gSimTrain2`: the five ranks of the card, rock and shell games.
pub(crate) const TRAINS: [[Train; 5]; 3] = [
    [
        train(5, 3, 1000, 0),
        train(6, 3, 3000, 11),
        train(7, 3, 5000, 21),
        train(7, 2, 8000, 31),
        train(7, 2, 10000, 41),
    ],
    [
        train(1, 0, 1000, 0),
        train(2, 0, 3000, 7),
        train(3, 1, 5000, 14),
        train(4, 1, 8000, 21),
        train(4, 2, 10000, 28),
    ],
    [
        train(5, 0, 1000, 0),
        train(5, 1, 3000, 7),
        train(8, 1, 5000, 14),
        train(8, 2, 8000, 21),
        train(10, 2, 10000, 28),
    ],
];

/// `gSimEventItems`: the items (0-based) an event may hand out, when the save owns them.
const EVENT_ITEMS: [usize; 42] = [
    0x22, 0x23, 0x26, 0x28, 0x2A, 0x2D, 0x2E, 0x30, 0x32, 0x33, 0x35, 0x37, 0x3A, 0x3E, 0x3F, 0x44,
    0x45, 0x49, 0x4B, 0x4C, 0x4D, 0x4E, 0x50, 0x53, 0x54, 0x57, 0x58, 0x5B, 0x5D, 0x5E, 0x60, 0x64,
    0x65, 0x66, 0x68, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70,
];
const ITEM_COUNT: usize = 0x15E;

/// A round of the run (section 25).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Round {
    pub announcer: i32,
    pub stage_change: i32,
    pub time_limit: i32,
    pub stage: i32,
    pub bgm: i32,
    pub change_allowed: i32,
    pub enemy: i32,
}

/// An opponent (section 26).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Enemy {
    pub chara: i32,
    pub color: i32,
    pub cpu_level: i32,
    pub last_item: i32,
    pub item: [i32; 7],
}

/// A level of the board (section 30): the highest attack and defence, and the points (/ 100) above which it is
/// reached.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Level {
    pub max: i32,
    pub need: i32,
}

/// A training's table (section 29): by times repeated the chance of each of six outcomes, and what each does.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TrainTable {
    pub weight: [[i32; 6]; 3],
    pub hp: [i32; 6],
    pub atk_min: [i32; 6],
    pub atk_max: [i32; 6],
    pub def_min: [i32; 6],
    pub def_max: [i32; 6],
}

/// `gSimDay->tex0` of `SimDay_Init`: the board (32 and 33 the monitor's pictures, 53 the fighter's icon).
const TEX0: [ImageTexture; 63] = [
    tex(3, 1),
    tex(3, 4),
    tex(3, 3),
    tex(3, 2),
    tex(3, 53),
    tex(31, 0),
    tex(3, 5),
    tex(3, 37),
    tex(3, 22),
    tex(3, 23),
    tex(3, 28),
    tex(3, 25),
    tex(3, 35),
    tex(3, 36),
    tex(3, 33),
    tex(3, 34),
    tex(3, 31),
    tex(3, 32),
    tex(3, 24),
    tex(3, 20),
    tex(3, 21),
    tex(3, 26),
    tex(3, 27),
    tex(3, 38),
    tex(3, 39),
    tex(3, 40),
    tex(3, 41),
    tex(3, 29),
    tex(3, 30),
    tex(3, 42),
    tex(3, 43),
    tex(3, 47),
    None,
    None,
    tex(3, 45),
    tex(3, 44),
    tex(3, 49),
    tex(3, 48),
    tex(3, 46),
    tex(3, 58),
    tex(3, 54),
    tex(3, 55),
    tex(3, 57),
    tex(3, 56),
    tex(3, 11),
    tex(3, 10),
    tex(3, 12),
    tex(3, 50),
    tex(3, 51),
    tex(3, 6),
    tex(3, 8),
    tex(3, 7),
    tex(3, 52),
    None,
    tex(3, 19),
    tex(3, 0),
    tex(3, 13),
    tex(3, 9),
    tex(3, 14),
    tex(3, 15),
    tex(3, 16),
    tex(3, 18),
    tex(3, 17),
];
/// `tex5`: the three-row window.
const TEX5: [ImageTexture; 6] = [
    tex(23, 1),
    tex(23, 2),
    None,
    tex(23, 0),
    tex(23, 3),
    tex(23, 4),
];
/// `tex1`: the event movie (7 the sign of a change, from section 22).
const TEX1: [ImageTexture; 11] = [
    tex(19, 7),
    tex(19, 6),
    tex(20, 0),
    tex(21, 0),
    tex(19, 2),
    tex(19, 4),
    tex(19, 3),
    tex(22, 0),
    tex(19, 5),
    tex(19, 0),
    tex(19, 1),
];
/// `tex2`..`tex4`: the card, shell and rock games.
const TEX2: [ImageTexture; 16] = [
    tex(16, 2),
    tex(16, 13),
    tex(17, 0),
    tex(16, 12),
    tex(17, 2),
    tex(17, 1),
    tex(16, 10),
    tex(16, 5),
    tex(16, 6),
    tex(16, 8),
    tex(16, 7),
    tex(16, 9),
    tex(16, 14),
    tex(16, 19),
    tex(17, 3),
    tex(16, 27),
];
const TEX3: [ImageTexture; 15] = [
    tex(16, 2),
    tex(16, 13),
    tex(17, 0),
    tex(16, 12),
    tex(17, 2),
    tex(17, 1),
    tex(16, 1),
    tex(16, 23),
    tex(16, 24),
    tex(16, 25),
    tex(16, 26),
    tex(16, 14),
    tex(16, 22),
    tex(16, 19),
    tex(17, 3),
];
const TEX4: [ImageTexture; 18] = [
    tex(16, 2),
    tex(16, 13),
    tex(17, 0),
    tex(16, 12),
    tex(17, 2),
    tex(17, 1),
    tex(16, 0),
    tex(16, 16),
    tex(16, 15),
    tex(16, 4),
    tex(17, 4),
    tex(16, 18),
    tex(16, 11),
    tex(16, 17),
    tex(16, 19),
    tex(17, 3),
    tex(16, 20),
    tex(17, 5),
];
/// `tex6`: the round picture (13 and 14 the two fighters' large pictures).
const TEX6: [ImageTexture; 16] = [
    tex(5, 2),
    tex(5, 3),
    tex(5, 4),
    tex(5, 5),
    tex(5, 6),
    tex(5, 7),
    tex(5, 8),
    tex(5, 9),
    tex(5, 12),
    tex(5, 0),
    tex(5, 1),
    tex(5, 10),
    tex(5, 11),
    None,
    None,
    tex(5, 13),
];
/// The files whose entries are picked at run time: the monitor's two picture sets (6, 7), the fighters' icons
/// (8) and the change signs (22).
const PICKED: [ImageTexture; 4] = [tex(6, 0), tex(7, 0), tex(8, 0), tex(22, 0)];
/// `MsgWin_Init(MPACK_AT(res, 9), …)`.
const MSG_TEXTURES: [ImageTexture; 3] = [Some(((9, 1), 0)), Some(((9, 1), 1)), None];

/// What `SimDay_Init` reads from its file.
pub struct SimDayAssets {
    pub data: SimDayData,
    pub background: Vec<RgbaTexture>,
    pub files: Vec<(FileKey, Vec<RgbaTexture>)>,
    pub dialog: DialogAssets,
}

#[derive(Clone, Debug)]
pub struct SimDayData {
    /// The movies: board 1, event 18, cards 10..12 (by the cards on the table), shells 13, rocks 14, window 15,
    /// round 2.
    pub board_fod: Vec<u8>,
    pub event_fod: Vec<u8>,
    pub card_fods: [Vec<u8>; 3],
    pub popo_fod: Vec<u8>,
    pub shot_fod: Vec<u8>,
    pub menu_fod: Vec<u8>,
    pub round_fod: Vec<u8>,
    pub msg_fod: Vec<u8>,
    /// Section 24: the board's lines (and the dialog's); 32: the item names.
    pub msg_text: Vec<u8>,
    pub text: Vec<u8>,
    pub dialog_fod: Vec<u8>,
    pub dialog_titles: Vec<u8>,
    pub(crate) events: Vec<[i32; 5]>,
    pub(crate) rounds: Vec<Round>,
    pub(crate) enemies: Vec<Enemy>,
    pub(crate) pools: Vec<[i32; 16]>,
    pub(crate) trains: Vec<TrainTable>,
    pub(crate) levels: Vec<Level>,
    /// Entries of the files of `PICKED`.
    pub picked: [usize; 4],
    pub layout: SimDayLayout,
}

#[derive(Clone, Debug, Default)]
pub struct SimDayLayout {
    pub background: usize,
    pub tex: [Vec<Option<usize>>; 7],
    pub msg: Vec<Option<usize>>,
    pub dialog: Vec<Option<usize>>,
    pub face_a: Vec<usize>,
    pub face_b: Vec<usize>,
    pub charas: Vec<usize>,
    pub signs: Vec<usize>,
    pub face_default: Option<usize>,
}

impl SimDayLayout {
    pub fn new(
        background: usize,
        dialog: Vec<Option<usize>>,
        picked: [usize; 4],
        first: impl Fn(FileKey) -> usize,
    ) -> Self {
        let i = |t: &[ImageTexture]| image_indices(t, &first);
        let entries = |n: usize| -> Vec<usize> {
            let (file, _) = PICKED[n].expect("a picked file");
            (0..picked[n]).map(|e| first(file) + 2 * e).collect()
        };
        Self {
            background,
            tex: [
                i(&TEX0),
                i(&TEX1),
                i(&TEX2),
                i(&TEX3),
                i(&TEX4),
                i(&TEX5),
                i(&TEX6),
            ],
            msg: i(&MSG_TEXTURES),
            dialog,
            face_a: entries(0),
            face_b: entries(1),
            charas: entries(2),
            signs: entries(3),
            face_default: i(&[tex(3, 59)])[0],
        }
    }
}

fn word(data: &[u8], at: usize) -> i32 {
    data.get(at..at + 4)
        .map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
}

/// Section `n` of a pack up to the start of section `n + 1` (the tables' sizes).
fn table(res: &[u8], n: usize) -> Result<&[u8], Box<dyn Error>> {
    let at = |k: usize| (word(res, k * 4) as u32 & !3) as usize;
    Ok(res
        .get(at(n)..at(n + 1))
        .ok_or("pack table out of bounds")?)
}

/// The records of a section, `size` bytes each.
fn records<T>(data: &[u8], size: usize, f: impl Fn(&[u8]) -> T) -> Vec<T> {
    data.chunks_exact(size).map(f).collect()
}

impl SimDayAssets {
    pub fn parse(res: &[u8]) -> Result<Self, Box<dyn Error>> {
        let files = decode_files(
            res,
            [
                &TEX0[..],
                &TEX1,
                &TEX2,
                &TEX3,
                &TEX4,
                &TEX5,
                &TEX6,
                &PICKED,
                &MSG_TEXTURES,
            ],
        )?;
        let count = |n: usize| {
            let (file, _) = PICKED[n].expect("a picked file");
            files
                .iter()
                .find(|(key, _)| *key == file)
                .map_or(0, |(_, t)| t.len())
        };
        let picked = [count(0), count(1), count(2), count(3)];
        let sec =
            |n: usize| -> Result<Vec<u8>, Box<dyn Error>> { Ok(pack_section(res, n)?.to_vec()) };
        let w = |r: &[u8], k: usize| word(r, k * 4);
        let six = |r: &[u8], k: usize| -> [i32; 6] { std::array::from_fn(|n| w(r, k + n)) };
        let dialog = DialogAssets::parse(pack_section(res, DIALOG_SECTION)?)?;
        Ok(Self {
            data: SimDayData {
                board_fod: sec(1)?,
                event_fod: sec(18)?,
                card_fods: [sec(10)?, sec(11)?, sec(12)?],
                popo_fod: sec(13)?,
                shot_fod: sec(14)?,
                menu_fod: sec(15)?,
                round_fod: sec(2)?,
                msg_fod: pack_section(pack_section(res, 9)?, 2)?.to_vec(),
                msg_text: sec(24)?,
                text: sec(32)?,
                dialog_fod: dialog.fod.clone(),
                dialog_titles: dialog.titles.clone(),
                events: records(table(res, 28)?, 0x14, |r| std::array::from_fn(|k| w(r, k))),
                rounds: records(table(res, 25)?, 0x1C, |r| Round {
                    announcer: w(r, 0),
                    stage_change: w(r, 1),
                    time_limit: w(r, 2),
                    stage: w(r, 3),
                    bgm: w(r, 4),
                    change_allowed: w(r, 5),
                    enemy: w(r, 6),
                }),
                enemies: records(table(res, 26)?, 0x2C, |r| Enemy {
                    chara: w(r, 0),
                    color: w(r, 1),
                    cpu_level: w(r, 2),
                    last_item: w(r, 3),
                    item: std::array::from_fn(|k| w(r, 4 + k)),
                }),
                pools: records(table(res, 27)?, 0x40, |r| std::array::from_fn(|k| w(r, k))),
                trains: records(table(res, 29)?, 0xC0, |r| TrainTable {
                    weight: [six(r, 0), six(r, 6), six(r, 12)],
                    hp: six(r, 18),
                    atk_min: six(r, 24),
                    atk_max: six(r, 30),
                    def_min: six(r, 36),
                    def_max: six(r, 42),
                }),
                levels: records(table(res, 30)?, 8, |r| Level {
                    max: w(r, 0),
                    need: w(r, 1),
                }),
                picked,
                layout: SimDayLayout::default(),
            },
            background: decode_picture(res, 4)?,
            files,
            dialog,
        })
    }
}

/// What a `SimDay_Run` ends in.
#[derive(Clone, Debug)]
pub enum SimDayResult {
    /// The round's fight was set up (mode 23 follows it).
    Battle(Box<BattleSetup>),
    /// The run was quit from the window (mode 13).
    Quit,
}

/// The stat changes an event queued (`SimChange`), shown one after the other.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Change {
    /// Bit n: stat n has a change pending; 0x10: the level-up is shown.
    pub mask: i32,
    pub applying: bool,
    pub delta: [i32; 4],
    pub stat: i32,
    pub shown: i32,
    pub mark: i32,
}

/// `SimDay` and the frame loop of `SimDay_Run`.
pub struct SimDayScreen {
    pub(crate) data: SimDayData,
    pub(crate) flash: [Flash; 7],
    tex: [Vec<Option<usize>>; 7],
    msg: MsgWin,
    dialog: Dialog,
    fade: ColorFade,
    pad: PadRepeat,
    lip: LipSync,
    pub(crate) flags: u32,
    load_state: i32,
    pub(crate) cur: [i32; 13],
    timer: i32,
    pub(crate) state: usize,
    prev_state: usize,
    talk: [i32; 2],
    pub(crate) msg_line: i32,
    menu_text: i32,
    bgm: u32,
    wait: i32,
    pub(crate) day: i32,
    own: Vec<usize>,
    event_set: usize,
    enemy_chara: i32,
    pub(crate) event: i32,
    /// `seq[0]`: the step of the running script; `seq[1]`: its frames.
    pub(crate) seq: i32,
    pub(crate) seq_timer: i32,
    last_event: i32,
    pub(crate) repeat: i32,
    pub(crate) face_a: i32,
    pub(crate) face_b: i32,
    pub(crate) rank: [i32; 3],
    pub(crate) answer: i32,
    pub(crate) chg: Change,
    potara: i32,
    level_up: bool,
    /// The run's state (`gProgress->sim`) and the player's fighter (`gProgress->member`).
    pub(crate) run: SimRun,
    chara: i32,
    color: i32,
    pub(crate) games: SimGames,
    pub(crate) sounds: Vec<TitleSound>,
    setup: Option<BattleSetup>,
    result: i32,
    done: bool,
}

/// `SimDay_GetTrainRank`: the rank (0..4) of training `kind` at turn `turn`.
fn train_rank(kind: usize, turn: i32) -> i32 {
    (0..4)
        .find(|&i| turn < TRAINS[kind][i + 1].turn - 1)
        .map_or(4, |i| i as i32)
}

impl SimDayScreen {
    /// `SimDay_Init(0)` and the first `ColorFade_StartIn`: `run` is the run's state, `chara` / `color` the
    /// player's fighter, `health` what the last fight left (read when the run is past its first turn).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: &SimDayData,
        run: SimRun,
        chara: i32,
        color: i32,
        health: f32,
        games: SimGames,
        save: &SaveData,
    ) -> Result<Self, Box<dyn Error>> {
        let rank = [0, 1, 2].map(|k| train_rank(k, run.turn));
        let make = |fod: &[u8]| -> Result<Flash, Box<dyn Error>> { Ok(Flash::create(fod)?) };
        let cards = (TRAINS[0][rank[0] as usize].param - 5).clamp(0, 2) as usize;
        let mut flash = [
            make(&data.board_fod)?,
            make(&data.event_fod)?,
            make(&data.card_fods[cards])?,
            make(&data.popo_fod)?,
            make(&data.shot_fod)?,
            make(&data.menu_fod)?,
            make(&data.round_fod)?,
        ];
        flash[FL_BOARD].play(1);
        flash[FL_BOARD].advance();
        flash[FL_MENU].play(1);
        flash[FL_EVENT].play(1);
        let mut tex = data.layout.tex.clone();
        tex[FL_BOARD][53] = data.layout.charas.get(chara.max(0) as usize).copied();
        let mut msg = MsgWin::new(
            &data.msg_fod,
            data.layout.msg.clone(),
            data.msg_text.clone(),
            true,
        )?;
        msg.open();
        let dialog = Dialog::new(
            &data.dialog_fod,
            data.dialog_titles.clone(),
            data.layout.dialog.clone(),
            Some(data.msg_text.clone()),
            0,
        )?;
        let mut screen = Self {
            data: data.clone(),
            flash,
            tex,
            msg,
            dialog,
            fade: ColorFade::start_in(true, FADE_FRAMES),
            pad: PadRepeat::default(),
            lip: LipSync::default(),
            flags: 0,
            load_state: LOAD_IDLE,
            cur: [0; 13],
            timer: 0,
            state: ST_BOARD,
            prev_state: ST_BOARD,
            talk: [0; 2],
            msg_line: -1,
            menu_text: 0,
            bgm: 0,
            wait: 0,
            day: 0,
            own: Vec::new(),
            event_set: 0,
            enemy_chara: 0,
            event: 0,
            seq: 0,
            seq_timer: 0,
            last_event: -1,
            repeat: 0,
            face_a: 0,
            face_b: 0,
            rank,
            answer: 0,
            chg: Change::default(),
            potara: 0,
            level_up: false,
            run,
            chara,
            color,
            games,
            sounds: Vec::new(),
            setup: None,
            result: 1,
            done: false,
        };
        screen.set_face_a(0);
        screen.set_face_b(0);
        if screen.run.turn != 0 {
            // Back from a fight: the health it left (at least 1), the turn of the round, the buttons.
            let mut hp = health as i32;
            if hp == 0 {
                hp = 1;
            }
            screen.run.stat[HP] = hp;
            screen.day = screen.run.turn % TURNS;
            screen.pass_wait(1);
            if screen.run.wait != 0 {
                screen.set_button(false, 3);
            } else {
                screen.set_button(true, 3);
            }
            if screen.run.stat[HP] < 21 {
                screen.set_button(false, 0);
                screen.set_button(false, 1);
                screen.cur[screen.state] = 2;
            }
        }
        screen.clip_goto(0, ST_BOARD, "fl_on_start");
        screen.list_items(save);
        let round = (screen.run.turn / TURNS).clamp(0, ROUND_BGM.len() as i32 - 1) as usize;
        screen.bgm = BGM_FIRST + ROUND_BGM[round];
        screen.play_bgm();
        Ok(screen)
    }

    /// `SimDay_Run`'s result once it has returned.
    pub fn result(&self) -> Option<SimDayResult> {
        if !self.done {
            return None;
        }
        Some(match (&self.setup, self.result) {
            (Some(setup), 1) => SimDayResult::Battle(Box::new(setup.clone())),
            _ => SimDayResult::Quit,
        })
    }

    /// The run's state as the screen leaves it (`SimDay_Run` clears the greyed-out buttons on the way out).
    pub fn run(&self) -> SimRun {
        let mut run = self.run;
        if self.done {
            run.off = 0;
        }
        run
    }

    /// The mini games' state (globals of the original that outlive the screen).
    pub fn games(&self) -> SimGames {
        self.games
    }

    /// `SimDay_SetFaceA`: picture `n` of the first set on the monitor (12 the default picture).
    pub(crate) fn set_face_a(&mut self, n: i32) {
        self.tex[FL_BOARD][32] = if n == 12 {
            self.data.layout.face_default
        } else {
            self.data.layout.face_a.get(n.max(0) as usize).copied()
        };
    }

    /// `SimDay_SetFaceB`.
    pub(crate) fn set_face_b(&mut self, n: i32) {
        self.tex[FL_BOARD][33] = self.data.layout.face_b.get(n.max(0) as usize).copied();
    }

    pub(crate) fn se(&mut self, id: u32) {
        self.sounds.push(TitleSound::Effect(2, id));
    }

    /// `SimDay_PlayBgm`.
    pub(crate) fn play_bgm(&mut self) {
        self.sounds.push(TitleSound::Bgm(self.bgm));
    }

    fn level(&self, n: i32) -> Level {
        self.data
            .levels
            .get(n.max(0) as usize)
            .copied()
            .unwrap_or(Level {
                max: 0,
                need: i32::MAX,
            })
    }

    /// `SimDay_SetupBattle`: the rule from the round table, the player's fighter with the board's attack and
    /// defence as two items and the carried items, the opponent from the opponent table.
    fn setup_battle(&mut self, random: &mut GameRandom, libc: &mut LibcRandom) {
        let round = (self.run.turn / TURNS).max(0) as usize;
        let rd = self.data.rounds.get(round).copied().unwrap_or_default();
        let en = self
            .data
            .enemies
            .get(rd.enemy.max(0) as usize)
            .copied()
            .unwrap_or_default();
        let announcer = if rd.announcer == RANDOM {
            random.range(8) as i32
        } else {
            rd.announcer
        };
        let stage = if rd.stage == RANDOM {
            random.range(0x23) as i32
        } else {
            rd.stage
        };
        let bgm = if rd.bgm == RANDOM { 0x18 } else { rd.bgm };
        // BattleSetup_SetRule: music 24 is a random one of 0..23.
        let bgm = if bgm == 24 { libc.rand() % 24 } else { bgm };
        self.enemy_chara = if en.chara == RANDOM {
            let pool = match round {
                3 => 1,
                5 => 2,
                _ => 0,
            };
            let chars = self.data.pools.get(pool).copied().unwrap_or([NONE; 16]);
            let n = chars.iter().take_while(|&&c| c != NONE).count();
            chars[random.range(n as u32) as usize]
        } else {
            en.chara
        };
        let mut items = [0u16; 8];
        items[0] = (self.run.stat[ATK] + 0xC4) as u16;
        items[1] = (self.run.stat[DEF] + 0x110) as u16;
        for i in 0..ITEM_NUM {
            let slot = (self.run.item_head as usize + i) % ITEM_NUM;
            if self.run.item[slot] != -1 {
                items[2 + i] = (self.run.item[slot] + 1) as u16;
            }
        }
        let mut enemy_items = [0u16; 8];
        let mut n = 0;
        for &id in &en.item {
            if id != NONE {
                enemy_items[n] = (id + 1) as u16;
                n += 1;
            }
        }
        enemy_items[7] = if en.last_item == NONE {
            0
        } else {
            (en.last_item + 1) as u16
        };
        let mut color = en.color;
        if self.enemy_chara == self.chara && self.color == color {
            color = i32::from(color == 0);
        }
        let start = |cpu_level: i32, health: f32| {
            Some(
                [MemberStart {
                    cpu_level,
                    health,
                    variant: 0,
                }; 5],
            )
        };
        let mut setup = BattleSetup {
            screen_mode: 0,
            mode: 2,
            bgm,
            time_limit: rd.time_limit,
            announcer,
            stage,
            stage_change: i32::from(rd.stage_change != 0),
            sides: [
                SideSetup {
                    control: 0,
                    pad: 0,
                    change_allowed: 1,
                    switch_enabled: 1,
                    chara: self.chara,
                    costume: self.color,
                    cpu_level: 0,
                    items,
                    members: start(0, self.run.stat[HP] as f32),
                    ..SideSetup::default()
                },
                SideSetup {
                    control: 2,
                    pad: 1,
                    change_allowed: i32::from(rd.change_allowed != 0),
                    switch_enabled: 1,
                    chara: self.enemy_chara,
                    costume: color,
                    cpu_level: en.cpu_level,
                    items: enemy_items,
                    members: start(en.cpu_level, 100.0),
                    ..SideSetup::default()
                },
            ],
            pool: Vec::new(),
            tutorial: 0,
            wide: true,
        };
        setup.finish();
        self.setup = Some(setup);
    }

    /// `SimDay_ListItems`: the owned items events may hand out, and the column of the event table (by those and
    /// whether every mission page is open).
    fn list_items(&mut self, save: &SaveData) {
        self.own = (0..ITEM_COUNT)
            .filter(|&i| save.item(i) & 1 != 0 && EVENT_ITEMS.contains(&i))
            .collect();
        let any = !self.own.is_empty();
        let all = save.mission_pages() == 20;
        self.event_set = match (all, any) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        };
    }

    /// `SimDay_AddChange`: queues a change of stat `stat` (points given in units, kept in hundreds).
    pub(crate) fn add_change(&mut self, stat: usize, amount: i32) {
        self.chg.mask |= 1 << stat;
        self.chg.delta[stat] = if stat == POINT { amount / 100 } else { amount };
    }

    /// `SimDay_GetStat(stat, 0)`: the run's stat (points in units).
    pub(crate) fn stat(&self, stat: usize) -> i32 {
        if stat == POINT {
            self.run.stat[POINT].wrapping_mul(100)
        } else {
            self.run.stat[stat]
        }
    }

    /// `SimDay_GiveItem`: one of the listed items at random replaces the oldest of the three carried.
    pub(crate) fn give_item(&mut self, random: &mut GameRandom) {
        let n = random.range(self.own.len() as u32) as usize;
        let head = self.run.item_head.rem_euclid(ITEM_NUM as i32) as usize;
        self.run.item[head] = self.own.get(n).map_or(0, |&i| i as i32);
        self.run.item_head = (self.run.item_head + 1) % ITEM_NUM as i32;
        self.cmd(2, random);
    }

    /// `SimDay_TakeItem`: the oldest carried item is taken away.
    pub(crate) fn take_item(&mut self, random: &mut GameRandom) {
        for i in 0..ITEM_NUM {
            let slot = (self.run.item_head as usize + i) % ITEM_NUM;
            if self.run.item[slot] != -1 {
                self.run.item[slot] = -1;
                self.cmd(3, random);
                return;
            }
        }
    }

    /// `SimDay_CountItems`.
    pub(crate) fn count_items(&self) -> i32 {
        self.run.item.iter().filter(|&&i| i != -1).count() as i32
    }

    /// `SimDay_SetButton`: opens or greys out a button of the board, when it is not so already.
    fn set_button(&mut self, on: bool, button: i32) {
        let f = &mut self.flash[FL_BOARD];
        let r = f.find(None, &format!("mc_sim_botan{button:02}"));
        let off = self.run.off >> button & 1 != 0;
        if on {
            if off {
                f.clip_goto_label(r, "fl_off_start");
                self.run.off &= !(1 << button);
            }
        } else if !off {
            f.clip_goto_label(r, "fl_no_select");
            self.run.off |= 1 << button;
        }
    }

    /// `SimDay_PassWait`.
    fn pass_wait(&mut self, turns: i32) {
        if self.run.wait != 0 {
            self.run.wait = (self.run.wait - turns).max(0);
        }
    }

    /// `SimDay_NextChange`: the first stat with a change pending.
    fn next_change(&self) -> Option<usize> {
        (0..4).find(|&i| self.chg.mask >> i & 1 != 0)
    }

    /// `SimDay_ShowChange`: the next pending change. Attack and defence go in steps of 1 or 2 drawn at random
    /// (the change stays pending until used up); health and points in one go.
    fn show_change(&mut self, random: &mut GameRandom) {
        let Some(stat) = self.next_change() else {
            return;
        };
        self.chg.stat = stat as i32;
        match stat {
            ATK | DEF => {
                let mut step = random.range(2) as i32 + 1;
                let delta = self.chg.delta[stat];
                if delta != 0 {
                    if delta > 0 {
                        step = step.min(delta);
                        self.chg.delta[stat] -= step;
                    } else {
                        let mut neg = -step;
                        if neg < delta {
                            step = -delta;
                            neg = delta;
                        }
                        self.chg.delta[stat] += step;
                        step = neg;
                    }
                    self.chg.shown = step + 2;
                } else {
                    self.chg.shown = 2;
                    self.chg.delta[stat] = 0;
                }
                self.chg.mark = random.range(3) as i32;
                if self.chg.delta[stat] == 0 {
                    self.chg.mask &= !(1 << stat);
                }
                self.cmd(0x1B, random);
            }
            HP => {
                self.cmd(0x1D, random);
                self.chg.shown = self.chg.delta[HP];
                self.chg.mask &= !4;
            }
            _ => {
                self.cmd(0x1F, random);
                self.chg.shown = self.chg.delta[POINT];
                self.chg.mask &= !8;
            }
        }
    }

    /// `SimDay_SetMenuAlpha`: the window shows its three item names or its three plates.
    fn set_menu_alpha(&mut self, text: bool) {
        let (name_alpha, plate_alpha) = if text { (1.0, 0.0) } else { (0.0, 1.0) };
        let f = &mut self.flash[FL_MENU];
        for i in 1..=3 {
            let r = f.find(None, &format!("mc_item_name_{i}"));
            f.clip_set_alpha(r, name_alpha);
        }
        for i in 1..=3 {
            let r = f.find(None, &format!("mc_menu_plate_{i}"));
            f.clip_set_alpha(r, plate_alpha);
        }
    }

    /// `SimDay_ClipGoto`: the plate under the cursor of state `kind` (board, trainings, two-row choice, window).
    fn clip_goto(&mut self, movie: usize, kind: usize, label: &str) {
        let name = match kind {
            ST_TRAIN => format!("mc_sim_botan{:02}", self.cur[ST_TRAIN] + 6),
            ST_SELECT => format!("mc_sim_botan{:02}", self.cur[ST_SELECT] + 4),
            ST_WINDOW => format!("mc_menu_plate_{}", self.cur[ST_WINDOW] + 1),
            _ => format!("mc_sim_botan{:02}", self.cur[ST_BOARD]),
        };
        let f = &mut self.flash[movie];
        let r = f.find(None, &name);
        f.clip_goto_label(r, label);
    }

    /// `SimDay_Cmd`: animation command `cmd` of the scripts. The number picks the movie: 0..18 the board, 19..23
    /// the window, 24..34 the event movie, 35..43 the cards, 44..53 the shells, 54..62 the rocks.
    pub(crate) fn cmd(&mut self, cmd: i32, random: &mut GameRandom) {
        let movie = match cmd {
            ..0x13 => FL_BOARD,
            0x13..0x18 => FL_MENU,
            0x18..0x23 => FL_EVENT,
            0x23..0x2C => FL_CARD,
            0x2C..0x36 => FL_POPO,
            _ => FL_SHOT,
        };
        let day = self.day;
        let level = self.run.level;
        let f = &mut self.flash[movie];
        let goto = |f: &mut Flash, parent: Option<&str>, clip: &str, label: &str| {
            let r = f.find(parent, clip);
            f.clip_goto_label(r, label);
        };
        match cmd {
            0 | 1 => {
                // The stars of the level reached.
                let label = if cmd == 0 {
                    "fl_on_start"
                } else {
                    "fl_on_loop"
                };
                for i in 0..=level {
                    let name = format!("mc_star_anime{:02}", 6 - i);
                    goto(f, Some("mc_chara_pt_plate"), &name, label);
                }
            }
            2 => {
                // Lights the first free item plate.
                if let Some(i) = (0..ITEM_NUM).find(|&i| self.potara >> i & 1 == 0) {
                    goto(f, None, &format!("mc_plate_potara{i}"), "fl_on_start");
                    self.potara |= 1 << i;
                }
            }
            3 => {
                // Puts out the last lit item plate.
                if let Some(i) = (0..ITEM_NUM).rev().find(|&i| self.potara >> i & 1 != 0) {
                    goto(f, None, &format!("mc_plate_potara{i}"), "fl_off_start");
                    self.potara &= !(1 << i);
                }
            }
            4 => goto(f, None, "mc_sim_monita", "fl_start"),
            5 => {
                let label = if self.flags & FACE_ON != 0 {
                    self.flags &= !FACE_ON;
                    "fl_chara_out"
                } else {
                    "fl_monita_off"
                };
                goto(f, None, "mc_sim_monita", label);
                self.flags |= BUSY;
            }
            6 => {
                self.set_face_b(self.face_b);
                let f = &mut self.flash[movie];
                goto(f, None, "mc_sim_monita", "fl_chara_in");
                self.flags |= FACE_ON;
            }
            7 => {
                goto(f, None, "mc_sim_monita", "fl_chara_only_out");
                self.flags &= !FACE_ON;
            }
            8 => {
                goto(f, None, "mc_icon_timer", "fl_start");
                goto(f, Some("mc_icon_timer"), "mc_icon_dragon_00", "fl_turn_on");
            }
            9 => {
                // The lamps of the turns played.
                for i in 0..=day {
                    let name = format!("mc_turn_lamp{i:02}");
                    goto(f, Some("mc_icon_timer"), &name, "fl_on_start");
                }
            }
            10 => {
                if day == TURNS - 1 {
                    goto(f, None, "mc_icon_timer", "fl_enemy_on");
                }
            }
            11 => {
                for i in 0..=day {
                    let name = format!("mc_turn_lamp{i:02}");
                    goto(f, Some("mc_icon_timer"), &name, "fl_red_on");
                }
                goto(f, Some("mc_icon_timer"), "mc_icon_dragon_00", "fl_enemy_on");
            }
            12 => f.goto_label("fl_botan_in", true),
            13 => f.goto_label("fl_botan_out", true),
            14 => {
                f.goto_label("fl_syugyo_botan_in", true);
                self.clip_goto(FL_BOARD, ST_TRAIN, "fl_off_start");
                self.cur[self.state] = 0;
                self.clip_goto(FL_BOARD, ST_TRAIN, "fl_on_start");
            }
            15 => f.goto_label("fl_syugyo_botan_out", true),
            16 => {
                f.goto_label("fl_syugyo_cancel", true);
                self.state = ST_BOARD;
            }
            17 => {
                self.state = ST_SELECT;
                f.goto_label("fl_select_in", true);
                self.clip_goto(FL_BOARD, ST_SELECT, "fl_off_start");
                self.cur[self.state] = 0;
                self.clip_goto(FL_BOARD, ST_SELECT, "fl_on_start");
            }
            18 => {
                f.goto_label("fl_select_out", true);
                self.state = ST_SCRIPT;
            }
            20 => {
                // Opens the window on the row it was left on.
                if self.state != ST_BACK {
                    self.prev_state = self.state;
                }
                self.state = ST_WINDOW;
                f.goto_label("fl_window_in", true);
                let cur = self.cur[ST_WINDOW];
                for i in 0..3 {
                    self.cur[ST_WINDOW] = i;
                    let label = if i == cur {
                        "fl_on_start"
                    } else {
                        "fl_off_start"
                    };
                    self.clip_goto(FL_MENU, ST_WINDOW, label);
                }
                self.cur[ST_WINDOW] = cur;
                self.flags |= WINDOW;
                self.sounds.push(TitleSound::Effect(1, 4));
                self.menu_text = 0;
            }
            21 => {
                f.goto_label("fl_window_out", true);
                self.sounds.push(TitleSound::Effect(1, 5));
            }
            22 => {
                f.goto_label("fl_wide", true);
                self.sounds.push(TitleSound::Effect(1, 4));
            }
            23 => {
                f.goto_label("fl_normal", true);
                self.sounds.push(TitleSound::Effect(1, 5));
            }
            25 => {
                goto(f, None, "mc_surprise_icon", "fl_start");
                self.state = ST_WAIT;
                self.se(0x19);
            }
            26 => {
                self.show_change(random);
                self.state = ST_WAIT;
            }
            27 => f.goto_label("fl_at_df_in", true),
            28 => {
                for i in 0..3 {
                    goto(f, None, &format!("mc_mark{i:02}"), "fl_start");
                }
            }
            29 => goto(f, None, "mc_pt_up_down", "fl_start"),
            30 => goto(f, None, "mc_pt_up_down", "fl_out"),
            31 => goto(f, None, "mc_score_up_down", "fl_start"),
            32 => goto(f, None, "mc_score_up_down", "fl_out"),
            33 => {
                // Level up: as many levels as the points allow.
                if self.run.level == LEVEL_MAX {
                    self.flags &= !LEVEL_UP;
                } else {
                    let mut i = self.run.level + 1;
                    loop {
                        self.run.level += 1;
                        i += 1;
                        if !(self.level(i).need < self.run.stat[POINT]
                            && self.run.level < LEVEL_MAX)
                        {
                            break;
                        }
                    }
                    self.chg.mask |= 0x10;
                    let f = &mut self.flash[movie];
                    goto(f, None, "mc_level_up", "fl_start");
                    self.msg_line = 0x57;
                    self.state = ST_WAIT;
                    self.flags |= LEVEL_UP;
                }
            }
            36 | 45 | 55 => f.play(1),
            37 | 38 => {
                let label = if cmd == 37 {
                    "fl_on_start"
                } else {
                    "fl_off_start"
                };
                for i in 0..TRAINS[0][self.rank[0].clamp(0, 4) as usize].param {
                    goto(f, None, &format!("mc_card_anime{i:02}"), label);
                }
            }
            39 => {
                let name = format!("mc_card_anime{:02}", self.cur[ST_SCRIPT]);
                goto(f, None, &name, "fl_on_maru_batsu");
            }
            40 | 50 | 59 => goto(f, None, "mc_win_text", "fl_start"),
            41 | 51 | 60 => goto(f, None, "mc_lose_text", "fl_start"),
            42 => f.goto_label("fl_card_out", true),
            43 | 53 | 62 => {
                f.reset_clips();
                f.stop();
            }
            46 => goto(f, None, "mc_fukidasi_popo", "fl_on"),
            47 => goto(f, None, "mc_fukidasi_popo", "fl_end"),
            48 => {
                for i in 0..5 {
                    goto(f, None, &format!("mc_popo{i:02}"), "fl_off_start");
                }
            }
            49 => {
                let name = format!("mc_popo{:02}", self.cur[self.state]);
                goto(f, None, &name, "fl_on_stop");
            }
            52 | 61 => f.goto_label("fl_game_out", true),
            56 => goto(f, None, "mc_shot", "fl_shot_fait"),
            57 => {
                goto(f, None, "mc_shot", "fl_shot_hit");
                goto(f, None, "mc_rock00", "fl_bom_start");
            }
            58 => {
                goto(f, None, "mc_shot", "fl_off");
                goto(f, None, "mc_rock00", "fl_off");
            }
            _ => {}
        }
    }

    /// `SimDay_PickEvent`: what the turn turns into from the button chosen: a training (scripts 0..2), a random
    /// event drawn by weight from the event table (5..36; 37, no script, when the column does not add up past the
    /// draw), the rest (3) or the full heal (4). Counts how often the same script came up in a row.
    fn pick_event(&mut self, random: &mut GameRandom) {
        let r = random.range(256) as i32;
        self.seq = 0;
        self.seq_timer = 0;
        match self.cur[ST_BOARD] {
            0 => self.event = self.cur[ST_TRAIN],
            1 => {
                let mut sum = 0;
                let mut i = 0;
                while i < EVENT_ROWS {
                    let w = self.data.events.get(i).map_or(0, |row| row[self.event_set]);
                    if w != 0 {
                        sum += w;
                        if r < sum {
                            break;
                        }
                    }
                    i += 1;
                }
                self.event = i as i32 + 5;
            }
            2 => self.event = 3,
            3 => self.event = 4,
            _ => {}
        }
        self.answer = 0;
        if self.event == self.last_event {
            self.repeat = (self.repeat + 1).min(2);
        } else {
            self.repeat = 0;
        }
        self.last_event = self.event;
        self.cur[ST_SCRIPT] = 0;
        self.msg_line = -1;
    }

    /// `SimDay_RefreshBoard`: the stars, the turn icon and the item plates, and a level-up the points allow.
    fn refresh_board(&mut self, random: &mut GameRandom) {
        self.cmd(0, random);
        self.cmd(8, random);
        for i in 0..ITEM_NUM {
            if self.run.item[i] != -1 {
                self.cmd(2, random);
            }
        }
        self.msg_line = self.cur[self.state] + 0x50;
        if self.run.level < LEVEL_MAX + 1
            && self.level(self.run.level + 1).need < self.run.stat[POINT]
        {
            self.level_up = true;
            self.cmd(0x21, random);
        }
    }

    /// One iteration of `SimDay_Run`'s loop.
    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        held: u32,
        pressed: u32,
        voice_stat: i32,
        save: &mut SaveData,
        loader: &mut dyn PictureLoader,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
        font: &mut Font,
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
        self.update_face_load(loader);
        self.update(random);
        self.update_talk(pressed, voice_stat);
        self.draw(font, &mut frame.quads);
        frame.quads.extend(self.fade.quad().map(ScreenQuad::from));
        if self.fade.is_in_done()
            && self.flags & GREETED == 0
            && self.flash[FL_BOARD].flags() & FLASH_PAD != 0
        {
            self.flags |= GREETED;
        }
        if self.fade.is_fading_out() {
            self.sounds.push(TitleSound::FadeOutStep);
        } else if self.fade.is_out_done() {
            if self.load_state == LOAD_IDLE {
                self.done = true;
            }
        } else if self.flags & LEAVING != 0 {
            if self.timer == 0 {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
            }
        } else if self.talk[0] == 0 {
            let pad = EventPad { pressed, repeat };
            self.input(pad, save, random, libc);
        }
        frame.sounds = std::mem::take(&mut self.sounds);
        frame
    }

    /// `SimDay_UpdateFaceLoad`: the two fighters' large pictures of the round picture, one step a frame.
    fn update_face_load(&mut self, loader: &mut dyn PictureLoader) {
        match self.load_state {
            LOAD_REQUEST => {
                self.tex[FL_ROUND][14] = None;
                self.tex[FL_ROUND][13] = None;
                self.load_state = LOAD_READ;
            }
            // File_UpdateRequests: the reads are done by the next call.
            LOAD_READ => self.load_state = LOAD_UNPACK,
            LOAD_UNPACK => {
                self.tex[FL_ROUND][14] = loader.portrait(FACE_FILE + self.chara.max(0) as u32);
                self.tex[FL_ROUND][13] =
                    loader.portrait(FACE_FILE + self.enemy_chara.max(0) as u32);
                self.flags |= FACES_READY;
                self.load_state = LOAD_IDLE;
            }
            _ => {}
        }
    }

    /// `SimDay_Update`: the movies' triggers (the turn passing, the monitor, the steps of a change, the window)
    /// and the change on show counted into its stat.
    fn update(&mut self, random: &mut GameRandom) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        let board = self.flash[FL_BOARD].trig();
        if board & 0x20 != 0 {
            self.cmd(4, random);
        }
        if board & 1 != 0 {
            match self.state {
                ST_BOARD => {
                    // The board is back: the turn is over.
                    if self.run.wait == 30 {
                        self.set_button(false, 3);
                    }
                    if self.flags & SAME_TURN != 0 {
                        self.flags &= !SAME_TURN;
                    } else if self.flags & SKIP_TURNS != 0 {
                        // Five turns pass, but not beyond the round's fight.
                        self.flags &= !SKIP_TURNS;
                        if self.day >= 5 {
                            self.run.turn = self.run.turn - self.day + 9;
                        } else {
                            self.run.turn += 5;
                        }
                        self.pass_wait(5);
                    } else {
                        self.run.turn += 1;
                        self.pass_wait(1);
                    }
                    self.day = self.run.turn % TURNS;
                    self.cmd(8, random);
                    self.cmd(12, random);
                    self.set_face_a(0);
                    self.msg_line = self.cur[self.state] + 0x50;
                    self.rank = [0, 1, 2].map(|k| train_rank(k, self.run.turn));
                }
                ST_SCRIPT => {
                    self.set_face_a(self.face_a);
                    self.set_face_b(self.face_b);
                }
                ST_VERSUS => {
                    self.set_face_a(12);
                    self.cmd(10, random);
                    self.msg_line = 0x58;
                }
                _ => {}
            }
            self.se(0x12);
        }
        if board & 4 != 0 {
            self.flags &= !BUSY;
        }
        if board & 8 != 0 {
            self.cmd(9, random);
        }
        if board & 2 != 0 {
            self.cmd(11, random);
        }
        let event = self.flash[FL_EVENT].trig();
        if event & 2 != 0 {
            match self.chg.stat as usize {
                ATK | DEF => {
                    // One step of attack or defence, kept within 0 and the level's maximum.
                    self.cmd(0x1C, random);
                    let stat = self.chg.stat as usize;
                    let max = self.level(self.run.level).max;
                    let v = (self.run.stat[stat] + self.chg.shown - 2).max(0);
                    self.run.stat[stat] = if max < v { max } else { v };
                    self.se(if self.chg.shown - 2 < 0 { 0x25 } else { 0x24 });
                }
                HP => {
                    self.se(if self.chg.delta[HP] < 0 { 0x1B } else { 0x1A });
                    self.chg.applying = true;
                }
                _ => {
                    self.se(0x1C);
                    self.chg.applying = true;
                }
            }
        }
        if event & 1 != 0 {
            if self.next_change().is_none() {
                self.state = ST_SCRIPT;
            } else {
                self.cmd(0x1A, random);
            }
        }
        if event & 4 != 0 {
            self.state = ST_SCRIPT;
        }
        if event & 8 != 0 {
            // The level-up animation ended.
            self.chg.mask &= !0x10;
            self.cmd(0, random);
            if self.flags & LEVEL_UP != 0 {
                if self.level_up {
                    self.state = ST_BOARD;
                    self.level_up = false;
                    self.msg_line = self.cur[self.state] + 0x50;
                } else {
                    self.state = ST_SCRIPT;
                }
                self.flags &= !LEVEL_UP;
            }
        }
        if self.flash[FL_MENU].trig() & 1 != 0 {
            self.menu_text = i32::from(self.state != ST_WINDOW);
        }
        if self.flash[FL_MENU].flags() & FLASH_END != 0 {
            // The window has closed: its row 2 asks whether to quit.
            if self.cur[self.state] == 2 {
                self.state = ST_CONFIRM;
                self.dialog.set_choices(true);
                self.dialog.set_cursor(1);
                let mut effects = Vec::new();
                self.dialog.start(Command::Open, &mut effects);
                self.push_effects(effects);
            } else {
                self.state = self.prev_state;
                self.flags &= !WINDOW;
            }
        }
        if self.chg.applying {
            self.apply_change(random);
        }
        // While the window is open only its movie runs.
        for i in 0..self.flash.len() {
            if self.flags & WINDOW == 0 || i == FL_MENU {
                self.flash[i].advance();
            }
        }
    }

    /// The change on show counted into its stat: health by 1 a frame within 20..100 (going up from 1), points
    /// by 7 within 0..65535.
    fn apply_change(&mut self, random: &mut GameRandom) {
        let stat = self.chg.stat.clamp(0, 3) as usize;
        let delta = self.chg.delta[stat];
        if delta > 0 {
            match stat {
                HP => {
                    if delta >= 2 {
                        self.run.stat[stat] += 1;
                        self.chg.delta[stat] -= 1;
                    } else {
                        self.run.stat[stat] += delta;
                        self.chg.delta[stat] = 0;
                    }
                    if self.run.stat[stat] > 100 {
                        self.run.stat[stat] = 100;
                    }
                    if self.run.stat[stat] > 20 {
                        // Above 20 the two buttons that need health open again.
                        if self.run.off & 1 != 0 {
                            self.set_button(true, 0);
                        }
                        if self.run.off & 2 != 0 {
                            self.set_button(true, 1);
                        }
                        if (self.cur[ST_BOARD] as u32) < 2 {
                            self.clip_goto(FL_BOARD, ST_BOARD, "fl_on_start");
                        }
                    }
                }
                POINT => {
                    if delta >= 8 {
                        self.run.stat[stat] += 7;
                        self.chg.delta[stat] -= 7;
                    } else {
                        self.run.stat[stat] += delta;
                        self.chg.delta[stat] = 0;
                    }
                    if self.run.stat[stat] > 0xFFFF {
                        self.run.stat[stat] = 0xFFFF;
                    }
                }
                _ => {}
            }
            if stat == POINT
                && self.run.level < LEVEL_MAX + 1
                && self.level(self.run.level + 1).need < self.run.stat[POINT]
            {
                self.cmd(0x21, random);
            }
        } else if delta < 0 {
            match stat {
                HP => {
                    if delta < -1 {
                        self.run.stat[stat] -= 1;
                        self.chg.delta[stat] += 1;
                    } else {
                        self.run.stat[stat] += delta;
                        self.chg.delta[stat] = 0;
                    }
                    if self.run.stat[stat] <= 0 {
                        self.run.stat[stat] = 1;
                    }
                    if self.run.stat[stat] < 20 {
                        self.run.stat[stat] = 20;
                    }
                    if self.run.stat[stat] < 21 {
                        // At 20 the two buttons that need health are greyed out and the cursor moves off them.
                        let mut moved = false;
                        if self.run.off & 1 == 0 {
                            self.set_button(false, 0);
                            moved = true;
                        }
                        if self.run.off & 2 == 0 {
                            self.set_button(false, 1);
                            moved = true;
                        }
                        if moved {
                            self.cur[ST_BOARD] = 2;
                            self.clip_goto(FL_BOARD, ST_BOARD, "fl_on_start");
                        }
                    }
                }
                POINT => {
                    if delta < -7 {
                        self.run.stat[stat] -= 7;
                        self.chg.delta[stat] += 7;
                    } else {
                        self.run.stat[stat] += delta;
                        self.chg.delta[stat] = 0;
                    }
                    if self.run.stat[stat] < 0 {
                        self.run.stat[stat] = 0;
                    }
                }
                _ => {}
            }
        }
        if self.chg.delta[stat] == 0 && self.chg.mask & 0x10 == 0 {
            match stat {
                HP => self.cmd(0x1E, random),
                POINT => self.cmd(0x20, random),
                _ => {}
            }
            self.chg.applying = false;
        }
    }

    fn push_effects(&mut self, effects: Vec<(u32, u32)>) {
        self.sounds.extend(
            effects
                .into_iter()
                .map(|(bank, id)| TitleSound::Effect(bank, id)),
        );
    }

    /// `SimDay_UpdateTalk`: the guide's closing line (or the next one, when a button cuts it short).
    fn update_talk(&mut self, pressed: u32, voice_stat: i32) {
        if self.talk[0] == 0 {
            return;
        }
        if pressed & PAD_CROSS != 0 {
            self.talk[0] += 1;
            self.sounds.push(TitleSound::Effect(1, 1));
        } else if pressed & PAD_TRIANGLE != 0 {
            self.talk[0] += 1;
            self.sounds.push(TitleSound::Effect(1, 2));
        } else if self.msg_line != -1 && voice_stat != VOICE_PLAYEND && self.talk[0] == self.talk[1]
        {
            return;
        }
        if (1..=5).contains(&self.talk[0]) {
            self.msg_line = 0x18 + self.talk[0];
        }
        anim::play_with_subtitle(
            &[],
            VOICE_BASE,
            self.msg_line,
            &mut self.lip,
            &mut self.sounds,
        );
        self.talk[0] = 0;
        self.msg_line = -1;
        self.talk[1] = self.talk[0];
    }

    /// `SimDay_Draw`: the status plates, the window's rows and item names, the change on show, the movies.
    fn draw(&mut self, font: &mut Font, quads: &mut Vec<ScreenQuad>) {
        quads.extend(
            picture_quads(self.data.layout.background)
                .into_iter()
                .map(ScreenQuad::from),
        );
        let run = self.run;
        let level_max = self.level(run.level).max;
        let f = &mut self.flash[FL_BOARD];
        let r = f.find(None, "mc_icon_timer");
        f.clip_set_flags(r, 0x400, true);
        let r = f.find(None, "mc_chara_pt_plate");
        f.clip_set_flags(r, 0x400, true);
        let child = |parent: &'static str, fmt: &'static str, first: i32| {
            move |n: i32| (parent.to_string(), format!("{fmt}{:02}", n + first))
        };
        anim::num_draw_child(
            f,
            child("mc_icon_timer", "mc_sim_time_num", 0),
            3,
            run.turn + 1,
            (0x20, 0x20),
            1,
        );
        let r = f.find(Some("mc_chara_pt_plate"), "mc_gauge_green");
        f.clip_set_flags(r, PROP_MASK, true);
        let r = f.find(Some("mc_chara_pt_plate"), "mc_gauge_red");
        f.clip_set_flags(r, PROP_MASK_SET, true);
        f.clip_set_offset(r, ((run.stat[HP] - 100) as f32 * 1.55) as i32, 0);
        anim::num_draw_child(
            f,
            child("mc_chara_pt_plate", "mc_status_num", 0),
            3,
            run.stat[HP],
            (0x20, 0x20),
            1,
        );
        for (plate, fmt, stat) in [
            ("mc_at_plate", "mc_at_num", ATK),
            ("mc_df_plate", "mc_df_num", DEF),
        ] {
            // The sign picture, the value, and the level's maximum.
            let sign = i32::from(run.stat[stat] < 0);
            for i in 0..2 {
                let r = f.find(Some(plate), &format!("{fmt}{:02}", i + 2));
                f.clip_set_tex(r, sign);
            }
            anim::num_draw_child(f, child(plate, fmt, 2), 2, run.stat[stat], (0x20, 0x20), 1);
            anim::num_draw_child(f, child(plate, fmt, 0), 2, level_max, (0x20, 0x20), 1);
        }
        anim::num_draw_child(
            f,
            child("mc_point_plate", "mc_score_num", 2),
            7,
            run.stat[POINT],
            (0x20, 0x20),
            1,
        );
        // The window: three rows cut from one texture; row 1 is dimmed when no item is carried.
        self.set_menu_alpha(self.menu_text != 0);
        let no_items = self.count_items() == 0;
        let f = &mut self.flash[FL_MENU];
        for i in 0..3 {
            let uv = Uv {
                u0: 0,
                v0: i << 5,
                u1: 0x200,
                v1: (i << 5) + 0x20,
                tex: 0,
            };
            let name = format!("mc_menu_plate_{}", i + 1);
            for clip in ["mc_menu_text_on", "mc_menu_text_off"] {
                let r = f.find(Some(&name), clip);
                f.clip_set_uv(r, uv);
                if i == 1 && no_items {
                    f.clip_set_color(r, 0.4);
                }
            }
        }
        // The names of the items carried, oldest first.
        let mut texts: Vec<TextDraw> = Vec::new();
        let names = TextBox::preset(0).with_align(1);
        let mut n = 0;
        for i in 0..ITEM_NUM {
            let slot = (run.item_head as usize + i) % ITEM_NUM;
            if run.item[slot] != -1 {
                let r = f.find(None, &format!("mc_item_name_{}", n + 1));
                texts.extend(names.attach_line(
                    f,
                    r,
                    0x400 + n as u32,
                    &self.data.text,
                    run.item[slot],
                    font,
                ));
                n += 1;
            }
        }
        // The change on show.
        let signs = &self.data.layout.signs;
        let f = &mut self.flash[FL_EVENT];
        match self.chg.stat as usize {
            ATK | DEF => {
                for i in 0..3 {
                    let name = format!("mc_mark{i:02}");
                    let r = f.find(None, &name);
                    if self.chg.mark == i {
                        f.clip_set_flags(r, PROP_VISIBLE, true);
                        f.clip_set_tex(r, self.chg.stat);
                        let r = f.find(Some(&name), "mc_pt_obj");
                        f.clip_set_tex(r, self.chg.shown);
                    } else {
                        f.clip_set_flags(r, PROP_VISIBLE, false);
                    }
                }
            }
            stat @ (HP | POINT) => {
                let (plate, fmt, arrow, sign, count, scale) = if stat == HP {
                    (
                        "mc_pt_up_down",
                        "mc_pt_num",
                        "mc_pt_yajirusi",
                        "mc_plus_minus",
                        5,
                        1,
                    )
                } else {
                    (
                        "mc_score_up_down",
                        "mc_score_num",
                        "mc_score_yajirusi",
                        "mc_score_plus_minus",
                        9,
                        100,
                    )
                };
                let r = f.find(None, plate);
                f.clip_set_flags(r, 0x400, true);
                anim::num_draw_child(
                    f,
                    child(plate, fmt, 0),
                    count,
                    self.chg.shown.wrapping_mul(scale),
                    (0x20, 0x20),
                    0,
                );
                if self.chg.shown < 0 {
                    let uv = Uv {
                        u0: 0x20,
                        v0: 0,
                        u1: 0x40,
                        v1: 0x20,
                        tex: 0,
                    };
                    for clip in [arrow, sign] {
                        let r = f.find(Some(plate), clip);
                        f.clip_set_uv(r, uv);
                    }
                    self.tex[FL_EVENT][7] = signs.first().copied();
                } else {
                    self.tex[FL_EVENT][7] = signs.get(1).copied();
                }
            }
            _ => {}
        }
        if self.state == ST_ROUND {
            anim::num_draw(
                &mut self.flash[FL_ROUND],
                |n| format!("mc_round_num{n:02}"),
                2,
                run.turn / TURNS + 1,
                (0x40, 0x40),
                1,
            );
        }
        // The message window goes between the fifth movie and the window's.
        for i in 0..self.flash.len() {
            let tex = &self.tex[i];
            let has_texture = |image: usize| tex.get(image).is_some_and(Option::is_some);
            let (fq, overs) = self.flash[i].draw_with_overs(&has_texture);
            quads.extend(screen_quads(&fq, tex));
            if i == FL_MENU {
                draw_texts(&texts, &overs, font);
                font.flush(quads);
            }
            if i == FL_SHOT {
                self.msg.draw(self.msg_line, font, quads);
            }
        }
        self.dialog.draw(true, font, quads);
    }

    /// `SimDay_Input`: the board's menus, the turn's script, the window, the announcement of the fight.
    fn input(
        &mut self,
        pad: EventPad,
        save: &mut SaveData,
        random: &mut GameRandom,
        libc: &mut LibcRandom,
    ) {
        let ok = pad.pressed & PAD_CROSS != 0;
        let cancel = pad.pressed & PAD_TRIANGLE != 0;
        let up = pad.repeat & PAD_UP != 0;
        let down = pad.repeat & PAD_DOWN != 0;
        let start = pad.pressed & PAD_START != 0;
        if self.flash[FL_BOARD].flags() & FLASH_PAD == 0 {
            return;
        }
        if (self.state == ST_WINDOW || self.state == ST_WINDOW_ITEMS)
            && self.flash[FL_MENU].flags() & FLASH_PAD == 0
        {
            return;
        }
        if self.flags & BUSY != 0 {
            return;
        }
        if self.flags & STARTED == 0 {
            self.refresh_board(random);
            self.flags |= STARTED;
        }
        if self.state == ST_BOARD && start {
            self.cmd(0x14, random);
            return;
        }
        let s = self.state;
        match s {
            ST_BOARD => {
                if up || down {
                    self.clip_goto(FL_BOARD, ST_BOARD, "fl_off_start");
                    loop {
                        self.cur[s] = if up {
                            if self.cur[s] - 1 < 0 {
                                3
                            } else {
                                self.cur[s] - 1
                            }
                        } else if self.cur[s] + 1 >= 4 {
                            0
                        } else {
                            self.cur[s] + 1
                        };
                        if self.run.off >> self.cur[s] & 1 == 0 {
                            break;
                        }
                    }
                    self.clip_goto(FL_BOARD, ST_BOARD, "fl_on_start");
                    self.sounds.push(TitleSound::Effect(1, 0));
                    self.msg_line = self.cur[s] + 0x50;
                } else if ok {
                    if self.cur[s] != 0 {
                        match self.cur[s] {
                            1 | 2 => self.pick_event(random),
                            3 => {
                                self.run.wait = 0x1E;
                                self.pick_event(random);
                                self.cur[s] = 0;
                                self.clip_goto(FL_BOARD, ST_BOARD, "fl_on_start");
                            }
                            _ => {}
                        }
                        self.cmd(0xD, random);
                        self.state = ST_SCRIPT;
                    } else {
                        self.state = ST_TRAIN;
                        self.cmd(0xE, random);
                        self.msg_line = self.cur[ST_TRAIN] + 0x54;
                    }
                    self.sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_TRAIN => {
                if up || down {
                    self.clip_goto(FL_BOARD, ST_TRAIN, "fl_off_start");
                    self.cur[s] = if up {
                        if self.cur[s] - 1 < 0 {
                            2
                        } else {
                            self.cur[s] - 1
                        }
                    } else if self.cur[s] + 1 >= 3 {
                        0
                    } else {
                        self.cur[s] + 1
                    };
                    self.clip_goto(FL_BOARD, ST_TRAIN, "fl_on_start");
                    self.sounds.push(TitleSound::Effect(1, 0));
                    self.msg_line = self.cur[s] + 0x54;
                } else if ok {
                    self.state = ST_SCRIPT;
                    self.cmd(0xF, random);
                    self.pick_event(random);
                    self.sounds.push(TitleSound::Effect(1, 1));
                } else if cancel {
                    self.cmd(0x10, random);
                    self.sounds.push(TitleSound::Effect(1, 2));
                    self.msg_line = self.cur[self.state] + 0x50;
                }
            }
            ST_WINDOW => {
                if up || down {
                    self.clip_goto(FL_MENU, ST_WINDOW, "fl_off_start");
                    if up {
                        self.cur[s] -= 1;
                        if self.cur[s] < 0 {
                            self.cur[s] = 2;
                        }
                        if self.cur[s] == 1 && self.count_items() == 0 {
                            self.cur[s] -= 1;
                        }
                    } else {
                        self.cur[s] += 1;
                        if self.cur[s] >= 3 {
                            self.cur[s] = 0;
                        }
                        if self.cur[s] == 1 && self.count_items() == 0 {
                            self.cur[s] += 1;
                        }
                    }
                    self.clip_goto(FL_MENU, ST_WINDOW, "fl_on_start");
                    self.sounds.push(TitleSound::Effect(1, 0));
                } else if ok {
                    let row = self.cur[s];
                    if row == 1 {
                        if self.count_items() != 0 {
                            self.cmd(0x16, random);
                            self.state = ST_WINDOW_ITEMS;
                            self.menu_text = row;
                            self.sounds.push(TitleSound::Effect(1, 1));
                        } else {
                            self.sounds.push(TitleSound::Effect(1, 7));
                        }
                    } else {
                        self.cmd(0x15, random);
                        self.sounds.push(TitleSound::Effect(1, 1));
                    }
                } else if start {
                    self.cur[s] = 0;
                    self.cmd(0x15, random);
                }
            }
            ST_WINDOW_ITEMS => {
                if ok {
                    self.menu_text = 0;
                    self.cmd(0x17, random);
                    self.state = ST_WINDOW;
                }
            }
            ST_CONFIRM => {
                self.dialog.set_msg(0xE);
                let mut effects = Vec::new();
                let answer = self.dialog.input(
                    false,
                    Pad {
                        pressed: pad.pressed,
                        repeat: pad.repeat,
                    },
                    &mut effects,
                );
                if answer != 0 {
                    self.state = if answer > 0 { ST_QUIT } else { ST_BACK };
                    self.dialog.set_choices(false);
                    self.dialog.start(Command::Close, &mut effects);
                }
                self.push_effects(effects);
            }
            ST_BACK => {
                if self.dialog.is_closed() {
                    self.cmd(0x14, random);
                }
            }
            ST_SELECT => {
                if up || down {
                    self.clip_goto(FL_BOARD, ST_SELECT, "fl_off_start");
                    self.cur[s] = if up {
                        if self.cur[s] - 1 < 0 {
                            1
                        } else {
                            self.cur[s] - 1
                        }
                    } else if self.cur[s] + 1 >= 2 {
                        0
                    } else {
                        self.cur[s] + 1
                    };
                    self.clip_goto(FL_BOARD, ST_SELECT, "fl_on_start");
                    self.sounds.push(TitleSound::Effect(1, 0));
                } else if ok {
                    self.answer = i32::from(self.cur[s] == 0);
                    self.cmd(0x12, random);
                    self.sounds.push(TitleSound::Effect(1, 1));
                }
            }
            ST_SCRIPT => {
                if self.flags & WAIT_KEY != 0 {
                    if ok {
                        self.flags &= !WAIT_KEY;
                        self.sounds.push(TitleSound::Effect(1, 1));
                    }
                } else if self.run_event(pad, save, random, libc) {
                    if self.day == TURNS - 1 {
                        self.state = ST_VERSUS;
                        self.setup_battle(random, libc);
                        self.load_state = LOAD_REQUEST;
                    } else {
                        self.state = ST_BOARD;
                    }
                    self.list_items(save);
                    self.cmd(5, random);
                }
            }
            ST_VERSUS => {
                if self.flags & FACES_READY == 0 {
                    return;
                }
                if ok {
                    self.flash[FL_ROUND].play(1);
                    self.se(0x14);
                    self.state = ST_ROUND;
                    self.sounds.push(TitleSound::Effect(1, 1));
                    self.talk[0] = if self.run.turn == LAST_TURN {
                        5
                    } else if self.chara == ANDROID_16 {
                        4
                    } else {
                        random.range(3) as i32 + 1
                    };
                }
            }
            ST_ROUND => {
                self.state = ST_ROUND_WAIT;
                self.wait = 300;
            }
            ST_ROUND_WAIT => {
                self.wait -= 1;
                if ok || self.wait == 0 {
                    self.sounds.push(TitleSound::BgmFadeOutStep);
                    self.sounds.push(TitleSound::VoiceStop);
                    self.flags |= 1 | LEAVING;
                    self.timer = 15;
                    if ok {
                        self.sounds.push(TitleSound::Effect(1, 1));
                    }
                }
            }
            ST_QUIT => {
                self.fade = ColorFade::start_out(true, FADE_FRAMES);
                self.result = 0;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoPictures;

    impl PictureLoader for NoPictures {
        fn portrait(&mut self, _file_id: u32) -> Option<usize> {
            Some(0)
        }

        fn picture(&mut self, _file_id: u32) -> Option<usize> {
            None
        }
    }

    fn real_data() -> Option<SimDayData> {
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};
        use bt3_data::{Iso9660, bpe};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let mut image = Iso9660::open(&iso).ok()?;
        let location = partition_entry_for_id(FILE_ID);
        let archive =
            AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition)).unwrap();
        let res = bpe::decode(&archive.read_entry(&mut image, location.index).unwrap()).unwrap();
        let assets = SimDayAssets::parse(&res).unwrap();
        let mut next = 100;
        let mut firsts = std::collections::BTreeMap::new();
        for (key, t) in &assets.files {
            firsts.insert(*key, next);
            next += 2 * t.len();
        }
        let mut data = assets.data;
        let dialog = vec![Some(1); assets.dialog.textures.len()];
        data.layout = SimDayLayout::new(0, dialog, data.picked, |k| firsts[&k]);
        Some(data)
    }

    struct Rig {
        screen: SimDayScreen,
        save: SaveData,
        random: GameRandom,
        libc: LibcRandom,
        font: Font,
        frames: u32,
    }

    impl Rig {
        fn new(data: &SimDayData, run: SimRun) -> Self {
            let save = SaveData::default();
            let screen =
                SimDayScreen::new(data, run, 3, 0, 100.0, SimGames::default(), &save).unwrap();
            Self {
                screen,
                save,
                random: GameRandom::from_key(&[7, 1]),
                libc: LibcRandom::new(1),
                font: Font::new(Vec::new(), 20),
                frames: 0,
            }
        }

        fn frame(&mut self, pressed: u32) {
            self.frames += 1;
            self.screen.run_frame(
                pressed,
                pressed,
                VOICE_PLAYEND,
                &mut self.save,
                &mut NoPictures,
                &mut self.random,
                &mut self.libc,
                &mut self.font,
            );
        }

        /// Frames without a button until the board takes the pad.
        fn ready(&mut self) -> bool {
            for _ in 0..400 {
                let s = &self.screen;
                if s.flags & STARTED != 0
                    && s.flags & BUSY == 0
                    && s.state == ST_BOARD
                    && s.flash[FL_BOARD].flags() & FLASH_PAD != 0
                {
                    return true;
                }
                self.frame(0);
            }
            false
        }

        /// Frames with the confirm button pressed every other frame, until `until` holds.
        fn confirm_until(&mut self, limit: u32, until: impl Fn(&SimDayScreen) -> bool) -> bool {
            for i in 0..limit {
                if until(&self.screen) {
                    return true;
                }
                self.frame(if i % 2 == 0 { PAD_CROSS } else { 0 });
            }
            until(&self.screen)
        }
    }

    /// Runs the real board from the ISO in `rom/`, when it is there: the tables, a rest on the first turn (half
    /// the health back, the turn passes), every event script to its end, and the round's fight.
    #[test]
    fn real_sim_day_plays_turns() {
        let Some(data) = real_data() else {
            return;
        };
        assert_eq!(data.events.len(), EVENT_ROWS);
        assert!(data.rounds.len() >= 7 && data.trains.len() == 3);
        assert!(data.levels.len() > LEVEL_MAX as usize + 1);
        // A rest on the first turn.
        let run = SimRun {
            stat: [0, 0, 50, 0],
            ..SimRun::default()
        };
        let mut rig = Rig::new(&data, run);
        assert!(rig.ready());
        rig.frame(PAD_DOWN);
        rig.frame(0);
        rig.frame(PAD_DOWN);
        rig.frame(0);
        assert_eq!(rig.screen.cur[ST_BOARD], 2);
        rig.frame(PAD_CROSS);
        assert_eq!((rig.screen.state, rig.screen.event), (ST_SCRIPT, 3));
        assert!(rig.confirm_until(3000, |s| s.state == ST_BOARD && s.run.turn == 1));
        assert!(rig.ready());
        let run = rig.screen.run();
        assert_eq!(run.stat[HP], 50 + 50 / 2 + 5);
        // Every script ends, its choices answered yes.
        for event in 0..37 {
            let mut rig = Rig::new(&data, SimRun::default());
            assert!(rig.ready());
            rig.screen.cmd(0xD, &mut rig.random);
            rig.screen.state = ST_SCRIPT;
            rig.screen.event = event;
            rig.screen.seq = 0;
            rig.screen.seq_timer = 0;
            assert!(
                rig.confirm_until(20000, |s| s.state == ST_BOARD),
                "event {event} stuck at step {} (state {})",
                rig.screen.seq,
                rig.screen.state
            );
        }
        // The tenth turn of the round: the fight.
        let run = SimRun {
            turn: 9,
            stat: [4, 3, 90, 120],
            item: [0x22, -1, -1],
            ..SimRun::default()
        };
        let mut rig = Rig::new(&data, run);
        rig.screen.day = 9;
        assert!(rig.ready());
        rig.frame(PAD_DOWN);
        rig.frame(0);
        rig.frame(PAD_DOWN);
        rig.frame(0);
        rig.frame(PAD_CROSS);
        assert!(rig.confirm_until(6000, |s| s.result().is_some()));
        let Some(SimDayResult::Battle(setup)) = rig.screen.result() else {
            panic!("no fight");
        };
        assert!(setup.wide);
        assert_eq!((setup.mode, setup.sides[0].chara), (2, 3));
        // The rest gave half the health + 5 (up to 100) and took 1 attack and 1 defence.
        assert_eq!(setup.sides[0].items[..3], [3 + 0xC4, 2 + 0x110, 0x23]);
        assert_eq!(setup.sides[0].members.unwrap()[0].health, 100.0);
    }

    #[test]
    fn the_trainings_rank_by_turn() {
        assert_eq!(train_rank(0, 0), 0);
        assert_eq!(train_rank(0, 9), 0);
        assert_eq!(train_rank(0, 10), 1);
        assert_eq!(train_rank(0, 40), 4);
        assert_eq!(train_rank(1, 5), 0);
        assert_eq!(train_rank(1, 6), 1);
        assert_eq!(train_rank(2, 27), 4);
    }
}
