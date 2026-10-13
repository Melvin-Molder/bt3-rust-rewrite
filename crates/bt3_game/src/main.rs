mod anim;
mod battle;
mod battle_seq;
mod boot_card;
mod btl_act;
mod btl_ai;
mod btl_blast;
mod btl_change;
mod btl_chr;
mod btl_clash;
mod btl_event;
mod btl_face;
mod btl_flags;
mod btl_fx;
mod btl_head;
mod btl_hit;
mod btl_input;
mod btl_ki_obj;
mod btl_menu;
mod btl_mgr;
mod btl_obj;
mod btl_obj_chain;
mod btl_partner;
mod btl_replay;
mod btl_script;
mod btl_snd;
mod btl_tech_disc;
mod btl_tech_fx;
mod btl_tech_ring;
mod btl_tech_rush;
mod btl_tech_shots;
mod btl_tech_sweep;
mod btl_tech_volley;
mod btl_text;
mod btl_vib;
mod char_ref;
mod char_select;
mod chr_cam;
mod chr_view;
mod clip;
mod dc_list;
mod dc_menu;
mod dc_password;
mod dc_save;
mod dialog;
mod draw;
mod duel_menu;
mod eft;
mod eft_absorb;
mod eft_aura;
mod eft_bill;
mod eft_bolt;
mod eft_bound;
mod eft_bubble;
mod eft_burst;
mod eft_chain;
mod eft_charge;
mod eft_flash;
mod eft_gdust;
mod eft_geyser;
mod eft_glow;
mod eft_impact;
mod eft_link;
mod eft_obj;
mod eft_orb_tail;
mod eft_part10;
mod eft_ptcl;
mod eft_quad;
mod eft_ray;
mod eft_rays;
mod eft_ribbon;
mod eft_rushburst;
mod eft_shock;
mod eft_shotfx;
mod eft_smoke;
mod eft_spark;
mod eft_spdline;
mod eft_storm;
mod eft_streak;
mod eft_surf;
mod eft_transform;
mod eft_water;
mod eft_weather;
mod evo_top;
mod evo_z;
mod fade;
mod flash;
mod font;
mod get_win;
mod gfx_water;
mod gsc;
mod hist_outro;
mod hist_result;
mod hist_save;
mod hist_select;
mod hud;
mod hud_combo;
mod hud_gauge;
mod hud_gs;
mod hud_prompt;
mod hud_team;
mod item_panel;
mod loading;
mod logo;
mod main_menu;
mod mcflow;
mod memcard;
mod menu_res;
mod mode_menu;
mod model_draw;
mod movie;
mod obj_shadow;
mod options;
mod pass_win;
mod password;
mod post;
mod post_draw;
mod replay_menu;
mod rng;
mod save;
mod scr_warp;
mod se;
mod shen;
mod shen_scene;
mod shop;
mod sound;
mod stage_anim;
mod stage_draw;
mod stage_fx;
mod stg_amb;
mod stg_blur;
mod stg_depth;
mod stg_glare;
mod stg_haze;
mod stg_obj;
mod stg_pan_blur;
mod stg_rigid;
mod stg_scroll;
mod stg_tint;
mod team_select;
mod title;
mod tour_menu;
mod training;
mod windows;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::asset::RenderAssetUsages;
use bevy::audio::{AddAudioSource, Volume};
use bevy::camera::visibility::RenderLayers;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite_render::AlphaMode2d;
use bevy::window::WindowResolution;
use bt3_data::Iso9660;
use bt3_data::adx;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::font::FontFile;
use bt3_data::gs::{RgbaTexture, decode_font_texture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use char_select::{
    BattleSetup, CharSelAssets, CharSelData, CharSelLayout, CharSelProgress, CharSelResult,
    CharSelScreen,
};
use draw::ScreenQuad;
use duel_menu::{
    DuelMenuAssets, DuelMenuData, DuelMenuLayout, DuelMenuResult, DuelMenuScreen, DuelProgress,
};
use loading::LoadingScreen;
use logo::LogoSequence;
use main_menu::{MainMenuAssets, MainMenuData, MainMenuLayout, MainMenuResult, MainMenuScreen};
use movie::{MoviePlayer, VideoStream};
use rng::{GameRandom, LibcRandom};
use title::{TextureLayout, TitleAssets, TitleData, TitleResult, TitleScreen, TitleSound};

const SCREEN_WIDTH: u32 = 512;
const SCREEN_HEIGHT: u32 = 448;
/// Game frames one update may run to catch up.
const MAX_CATCH_UP: f32 = 2.0;
const FIRST_LOADING_FILE_ID: u32 = 0x3C1;
// gProgress->baseFile (0x1C1) + 0x19: menu archive 0, loaded by FirstRun_Main.
const LOGO_ARCHIVE_FILE_ID: u32 = 0x1C1 + 0x19;
const COMMON_SE_BANK_FILE_ID: u32 = 0x14B;
const MENU_SE_BANK_FILE_ID: u32 = 0x14C;
/// `COMMON_FILE_BOOT`: the two fonts and the text icons.
const BOOT_FILE_ID: u32 = 5;
// NTSC: one vertical blank is 1001/60000 s.
const VBLANK_SECONDS: f32 = 1001.0 / 60000.0;

enum Scene {
    /// `BootCard_Run`: the memory card check before the logos.
    BootCard(Box<boot_card::BootCardScreen>),
    /// `Dc_Main` mode 53: the Data Center's top menu.
    DcMenu(Box<dc_menu::DcMenuScreen>),
    /// `CharRefMode_Main`, progress mode 60: the character reference.
    CharRef(Box<char_ref::CharRefScreen>),
    /// `OptMode_Main`, progress mode 62: the options screen.
    Options(Box<options::OptionScreen>),
    /// `TrainMode_Main`, progress mode 44: the training menu.
    Training(Box<training::TrainScreen>),
    /// `EvoMode_Main` mode 48: Evolution Z's top menu.
    EvoTop(Box<evo_top::EvoTopScreen>),
    /// `Tour_Main` mode 33: Dragon World Tour's menu.
    TourMenu(Box<tour_menu::TourMenuScreen>),
    /// `EvoMode_Main` mode 49: the customising screen.
    EvoZ(Box<evo_z::EvoZScreen>),
    /// `EvoMode_Main` mode 50: the item shop.
    Shop(Box<shop::ShopScreen>),
    /// `Shen_Main` mode 70: the wish screen; mode 71: its save screen.
    Shen(Box<shen::ShenScreen>),
    ShenSave(Box<shen::ShenSaveScreen>),
    /// `Dc_Main` mode 54: the password screen.
    DcPass(Box<dc_password::DcPassScreen>),
    /// `Dc_Main` mode 55: the custom character list.
    DcList(Box<dc_list::DcListScreen>),
    /// `Dc_Main` mode 56: the replay list.
    ReplayMenu(Box<replay_menu::ReplayMenuScreen>),
    Logos(LogoSequence),
    /// `Movie_Run`: the opening movie (`false`) or the story's ending movie (`true`).
    Movie(MoviePlayer, VideoStream, bool),
    Title(Box<TitleScreen>),
    MainMenu(Box<MainMenuScreen>),
    DuelMenu(Box<DuelMenuScreen>),
    CharSelect(Box<CharSelScreen>),
    /// `TeamSel_Run(2)`, progress mode 40.
    TeamSelect(Box<team_select::TeamSelScreen>),
    /// `HistSel_Run(1)`, progress mode 6: the story mode's saga select.
    HistSelect(Box<hist_select::HistSelScreen>),
    /// `ModeMenu_Run(2)`, progress mode 7: a saga's episode list.
    ModeMenu(Box<mode_menu::ModeMenuScreen>),
    /// `HistResult_Run(3)`, progress mode 8: the story battle's result.
    HistResult(Box<hist_result::HistResultScreen>),
    /// `HistSave_Run(4)`, progress mode 10: the story mode's save.
    HistSave(Box<hist_save::HistSaveScreen>),
    /// `HistOutro_Run(2)`, progress mode 9: a completed saga's outro.
    HistOutro(Box<hist_outro::HistOutroScreen>),
    Black,
    Loading(LoadingScreen),
    /// `Battle_Load`: the loading screen while the battle's files are read.
    BattleLoad(Box<BattleLoad>),
    Battle(Box<battle::Battle>),
}

/// `Load_RunBlocking` inside `Battle_Load`.
struct BattleLoad {
    screen: LoadingScreen,
    /// Where the loading screen's textures start.
    texture_base: usize,
    job: Option<JoinHandle<Result<battle::BattleAssets, String>>>,
    setup: BattleSetup,
}

impl Scene {
    /// `BootCard_Run`, `Logo_Show`, `Title_Run` and `MainMenu_Run` end their frames with `Gfx_EndFrame(1)`; `Movie_Run` and
    /// `Load_RunBlocking` use 2.
    fn frame_seconds(&self) -> f32 {
        match self {
            Scene::BootCard(_)
            | Scene::DcMenu(_)
            | Scene::DcPass(_)
            | Scene::DcList(_)
            | Scene::ReplayMenu(_)
            | Scene::Logos(_)
            | Scene::Title(_)
            | Scene::MainMenu(_)
            | Scene::DuelMenu(_)
            | Scene::CharSelect(_)
            | Scene::TeamSelect(_)
            | Scene::HistSelect(_)
            | Scene::ModeMenu(_)
            | Scene::HistResult(_)
            | Scene::HistSave(_)
            | Scene::HistOutro(_)
            | Scene::Options(_)
            | Scene::Training(_)
            | Scene::EvoTop(_)
            | Scene::Shop(_)
            | Scene::EvoZ(_)
            | Scene::TourMenu(_)
            | Scene::ShenSave(_)
            | Scene::CharRef(_)
            | Scene::Black => VBLANK_SECONDS,
            // Shen_Run: Gfx_EndFrame(2).
            Scene::Shen(_)
            | Scene::Movie(..)
            | Scene::Loading(_)
            | Scene::BattleLoad(_)
            | Scene::Battle(_) => 2.0 * VBLANK_SECONDS,
        }
    }
}

/// A WAV decoded from an ADX file on a worker thread.
type Decoding = JoinHandle<Result<Vec<u8>, String>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sampling {
    /// `Sprite_DrawPicture` and the loading screens.
    Nearest,
    /// The Flash player's TEX1 (bilinear), clamped.
    Linear,
    /// Bilinear, wrapping (`FLASH_QUAD_REPEAT`).
    LinearRepeat,
}

/// An ADXT player that is sounding: its entity and output volume in 0.1 dB.
#[derive(Clone, Copy)]
struct Player {
    entity: Entity,
    level: i32,
}

#[derive(Resource)]
struct Game {
    scene: Scene,
    textures: Vec<RgbaTexture>,
    samplers: Vec<Sampling>,
    accumulator: f32,
    pressed_latch: u32,
    /// Port 2's menu buttons pressed since the last frame.
    pressed_latch2: u32,
    /// `PAD_*` bits pressed since the last battle frame, per port.
    battle_latch: [u32; 2],
    quads: Vec<ScreenQuad>,
    iso_path: PathBuf,
    random: GameRandom,
    /// newlib `rand()`, behind `Rand_IntRange`.
    libc_random: LibcRandom,
    /// The VU0 R register (`Rand_FloatRange`), seeded at boot by `Vu0_Init`; a battle holds it while it runs.
    vu0_random: rng::Vu0Random,
    /// The loading screen type shown last (`Load_InitScreen` avoids repeating it).
    last_loading_type: Option<u16>,
    /// The battle's model textures (material, shading rows) and this frame's model triangles.
    model_textures: (Vec<RgbaTexture>, Vec<RgbaTexture>),
    /// Bumped whenever `model_textures` starts over (a battle, a model of the viewer).
    model_rev: u32,
    model_batches: Vec<battle::ModelBatch>,
    /// The stage texture file (a new revision after a stage change) and this frame's stage triangles.
    stage_textures: Vec<RgbaTexture>,
    stage_rev: u32,
    stage_batches: Vec<stage_draw::StageBatch>,
    /// The fighters' effect textures and this frame's effect triangles in draw order.
    effect_textures: Vec<RgbaTexture>,
    effects: Vec<eft::EffectTri>,
    /// What `StgFx_DrawPre` draws between the stage and the fighters (the back tints).
    under_effects: Vec<eft::EffectTri>,
    /// The fighters' ground shadows, after the fighters.
    shadow_effects: Vec<eft::EffectTri>,
    /// The pads' vibration of the last battle frame (`Pad.vibData`), and what pad 0's gamepads were last sent.
    rumble: [u8; 2],
    rumble_sent: [u8; 2],
    /// Effect textures sampled point by point (TEX1 0: the shadow pages).
    effect_point: Vec<usize>,
    /// `StgFx_DrawPost`'s redraws of the frame (the radial blur).
    post_ops: Vec<post::PostOp>,
    /// `StgFx_DrawPre`'s ops, between the stage and the fighters.
    pre_ops: Vec<post::PostOp>,
    /// The battle frame's depth tint (`StgDepthTint_Draw`).
    depth_tint: Option<stg_depth::DepthClutDraw>,
    /// The effect textures whose texels changed this frame (the draw writes them again).
    effect_updates: Vec<usize>,
    /// The HUD sheets of the battle and this frame's HUD sprites.
    hud_textures: Vec<RgbaTexture>,
    hud_prims: Vec<hud::HudPrim>,
    /// The gauges and timer, drawn on the CPU (premultiplied RGBA, 512 x 448).
    hud_layer: Option<Vec<u8>>,
    /// `ScrXfade`: frames captured so far, and the captured frame's alpha (0..128) over this one.
    xfade_captures: u32,
    xfade_alpha: i32,
    /// HUD textures from this index on (the menu sheet) clamp at their edges (`CLAMP_1` 1 of the default
    /// environment); the others wrap.
    hud_clamp_from: usize,
    /// `gProgress->demoCount` / `demoPick`.
    demo: char_select::DemoProgress,
    /// `gProgress->mode` is the title's (1) while the attract battle runs: the title follows it.
    in_demo: bool,
    /// `gProgress->mode` is 8 while a story battle runs: its result screen follows it.
    story_battle: bool,
    /// The story battle's script, set up and waiting for the battle to load.
    pending_script: Option<btl_script::BtlScript>,
    /// `gProgress->prevMode` when the ending movie started: 6 the saga select, 9 an outro.
    ending_from: i32,
    /// `BattleResult.dragonBallFound` of the story battle: 1 + the dragon ball, 0 none.
    story_ball: i32,
    movie_texture: Option<usize>,
    soundtrack: Option<Decoding>,
    soundtrack_source: Option<Handle<AudioSource>>,
    music: Option<Decoding>,
    music_source: Option<Handle<AudioSource>>,
    title: Option<TitleData>,
    main_menu: Option<MainMenuData>,
    duel_menu: Option<DuelMenuData>,
    /// The duel fields of `gProgress`.
    duel: DuelProgress,
    hist_select: Option<hist_select::HistSelData>,
    /// The story mode's fields of `gProgress`.
    hist: hist_select::HistProgress,
    /// The episode list's own section and the first texture of each of its files; the sagas' packs, by saga.
    mode_menu: Option<(mode_menu::ModeMenuData, HashMap<menu_res::FileKey, usize>)>,
    sagas: HashMap<usize, mode_menu::SagaData>,
    hist_result: Option<hist_result::HistResultData>,
    /// `--story-result win|lose`: start at the story battle's result screen.
    story_result: Option<bool>,
    char_select: Option<CharSelData>,
    /// The character select's fields of `gProgress`.
    char_progress: CharSelProgress,
    /// Common file 4's character and item tables (`ChrTbl_*`, `ItemTbl_*`).
    chr_table: bt3_data::chara_data::ChrTable,
    /// Portraits and stage pictures by file: their first texture.
    pictures: HashMap<u32, usize>,
    save: save::SaveData,
    /// The memory card (`gMcCardPort` over the emulated libmc) and the parts of `gProgress` its flows use.
    card: memcard::McCard,
    progress: mcflow::Progress,
    /// `McFlow_Init`'s resources: common file base + 0x1D, unpacked (the save icons).
    card_res: Option<Vec<u8>>,
    /// Common file 4: the character and item tables (a saved replay's members' derived values).
    chara_table: Vec<u8>,
    /// The Data Center's screens (archive 8, sections 1 and 4) and its fields of `gProgress`.
    dc_menu: Option<dc_menu::DcMenuData>,
    options: Option<options::OptionData>,
    char_ref: Option<char_ref::CharRefData>,
    /// `gMenuArc6`'s training menu (section 1) and its character select (section 2).
    train: Option<training::TrainData>,
    train_char_select: Option<CharSelData>,
    /// The progress mode (44 the menu, 45 its character select) of the training screen that set up the battle,
    /// or that the character select runs in.
    train_mode: Option<i32>,
    /// `gMenuArc12`'s wish screen, and whether its backdrop's textures went to the renderer.
    shen: Option<shen::ShenData>,
    /// `gMenuArc7`'s top menu of Evolution Z; `gProgress + 0x7D0`, the plate it was left on (-1 when the mode is
    /// entered); whether `EvoMode_Main` started its music.
    evo_top: Option<evo_top::EvoTopData>,
    evo_cursor: i32,
    evo_bgm: bool,
    shop: Option<shop::ShopData>,
    evo_z: Option<evo_z::EvoZData>,
    /// `gMenuArc4`'s tournament menu, and the tournament's block of `gProgress`.
    tour_menu: Option<tour_menu::TourMenuData>,
    tour: tour_menu::TourSession,
    shen_textures: bool,
    /// The character reference's model viewer (`ChrView`) while the screen runs, and the model texture revision
    /// of it the renderer has.
    chr_view: Option<chr_view::ChrView>,
    chr_view_rev: Option<u32>,
    /// Pad 0's sticks as `Pad_Update` gives them to the menus (`gPad[0].left`, `.right`).
    menu_sticks: [[f32; 2]; 2],
    replay_menu: Option<replay_menu::ReplayMenuData>,
    dc_pass: Option<dc_password::DcPassData>,
    dc_list: Option<dc_list::DcListData>,
    dc: dc_menu::DcProgress,
    /// The last battle's setup and replay (`gBattleReplay`), which the replay list saves.
    last_battle: Option<(BattleSetup, btl_replay::Replay)>,
    /// A replay loaded from the memory card for the battle being set up; the battle plays it.
    pending_replay: Option<btl_replay::Replay>,
    /// `gProgress->mode` is the replay list's (56): the battle came from it and goes back to it.
    in_replay_list: bool,
    /// Menu music by file, once decoded, and the file being decoded.
    bgm_sources: HashMap<u32, Handle<AudioSource>>,
    pending_bgm: Option<(u32, Decoding)>,
    /// `gProgress->flags & MPROG_SAVE_LOADED`: set once the title was left for the main menu.
    save_loaded: bool,
    /// `gProgress->mainMenuItem`.
    main_menu_item: i32,
    bgm: Option<Player>,
    /// The second music player ("MAP", the stage's ambience), the file being decoded for it, and its volume.
    map_bgm: Option<Player>,
    pending_map: Option<Decoding>,
    map_volume: i32,
    /// Voice players 0 and 1.
    voice: [Option<Player>; 2],
    /// Streamed-SE players 0 and 1 and the files being decoded for them, with their volumes.
    stream_se: [Option<Player>; 2],
    pending_stream_se: [Option<(Decoding, i32)>; 2],
    /// `Adx_PauseSeVoice`: the voice and streamed-SE players are paused.
    se_voice_paused: bool,
    /// The voice entity was spawned this update, so it is not in the world yet.
    voice_fresh: bool,
    /// The same for streamed-SE player 0, and whether its last file played to its end.
    stream_fresh: bool,
    stream_ended: bool,
    /// The last voice played to its end (`ADXT_STAT_PLAYEND`).
    voice_ended: bool,
    /// The files being decoded for the voice players, with their volumes.
    pending_voice: [Option<(Decoding, i32)>; 2],
    picture: Option<Vec<u8>>,
    /// `--title`: start at Progress_Main mode 1 instead of FirstRun_Main.
    skip_to_title: bool,
    /// `--menu`: start at Progress_Main mode 4.
    skip_to_menu: bool,
    /// `--battle`: start straight at `Battle_Main` with characters 0 and 1 on stage 0.
    skip_to_battle: bool,
    /// `--stage`: the quick battle's stage.
    battle_stage: i32,
    /// `--mode N` (`--course C` for survival): the battle mode `--battle` starts in.
    battle_mode: (i32, usize),
    /// `--split`: `--battle` is 2P versus in split screen.
    battle_split: bool,
    se: se::SoundEffects,
    font: font::Font,
}

#[derive(Resource, Default)]
struct Textures(Vec<Handle<Image>>);

#[derive(Component)]
struct QuadSlot(usize);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image_path =
        PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
    let mut forced_type = None;
    let mut skip_to_title = false;
    let mut skip_to_menu = false;
    let mut story_result = None;
    let mut skip_to_battle = false;
    let mut battle_stage = 0;
    let mut seed = None;
    let mut battle_mode = (0, 0);
    let mut memcard_dir = PathBuf::from("memcard");
    let mut battle_split = false;
    let mut log_fps = false;
    let mut shot = None;
    let mut shot_at = 600;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--title" {
            skip_to_title = true;
        } else if argument == "--menu" {
            skip_to_title = true;
            skip_to_menu = true;
        } else if argument == "--story-result" {
            // A debugging aid while story battles are not ported: their result screen, saga 0's first episode.
            story_result = Some(
                match arguments
                    .next()
                    .map(|v| v.to_string_lossy().into_owned())
                    .as_deref()
                {
                    Some("win") => true,
                    Some("lose") => false,
                    _ => return Err("--story-result requires win or lose".into()),
                },
            );
            skip_to_title = true;
        } else if argument == "--fps" {
            log_fps = true;
        } else if argument == "--shot" {
            // A debugging aid: the window saved as a PNG after `--shot-at` updates, then the program ends.
            shot = Some(PathBuf::from(
                arguments.next().ok_or("--shot requires a file")?,
            ));
        } else if argument == "--stage" {
            battle_stage = arguments
                .next()
                .and_then(|value| value.to_string_lossy().parse::<i32>().ok())
                .filter(|value| (0..0x23).contains(value))
                .ok_or("--stage requires a stage of 0 to 34")?;
            skip_to_title = true;
            skip_to_battle = true;
        } else if argument == "--seed" {
            // The hardware timer's value Rand_Init seeds from, for runs that repeat.
            seed = Some(
                arguments
                    .next()
                    .and_then(|value| value.to_string_lossy().parse::<u32>().ok())
                    .ok_or("--seed requires a number")?,
            );
        } else if argument == "--shot-at" {
            shot_at = arguments
                .next()
                .and_then(|value| value.to_string_lossy().parse::<u32>().ok())
                .ok_or("--shot-at requires a number of updates")?;
        } else if argument == "--battle" {
            skip_to_title = true;
            skip_to_battle = true;
        } else if argument == "--mode" {
            // Story mode (1) needs its script, which is not ported; nothing sets up 8 or 9.
            battle_mode.0 = arguments
                .next()
                .and_then(|value| value.to_string_lossy().parse::<i32>().ok())
                .filter(|value| matches!(value, 0 | 2..=7))
                .ok_or("--mode requires a battle mode of 0 or 2 to 7")?;
            skip_to_title = true;
            skip_to_battle = true;
        } else if argument == "--memcard" {
            memcard_dir = PathBuf::from(arguments.next().ok_or("--memcard requires a folder")?);
        } else if argument == "--split" {
            battle_split = true;
            skip_to_title = true;
            skip_to_battle = true;
        } else if argument == "--course" {
            battle_mode.1 = arguments
                .next()
                .and_then(|value| value.to_string_lossy().parse::<usize>().ok())
                .filter(|value| *value < bt3_data::survival::COURSES)
                .ok_or("--course requires a survival course of 0 to 2")?;
        } else if argument == "--screen" {
            let value = arguments
                .next()
                .and_then(|value| value.to_string_lossy().parse::<u16>().ok())
                .filter(|value| *value <= 2)
                .ok_or("--screen requires a value of 0, 1, or 2")?;
            forced_type = Some(value);
        } else {
            image_path = PathBuf::from(argument);
        }
    }

    // Rand_Init seeds from the hardware timer, so the original's picks also differ on every boot.
    let timer = match seed {
        Some(seed) => seed,
        None => SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos(),
    };
    let (game_random, libc_random) = rng::init_from_timer(timer);
    let mut image = Iso9660::open(&image_path)?;
    // Save_Init's defaults need the item table (common file 4); McFlow_Init's icons are common file base + 0x1D.
    let chara_table = read_file(&mut image, 4).unwrap_or_default();
    let chr_table = bt3_data::chara_data::ChrTable::parse(&chara_table, 0xA1);
    let new_save = save::SaveData::new(&chr_table.items);
    let card_res = read_file(&mut image, duel_menu::ARCHIVE_FILE_ID + 0x1D)
        .ok()
        .and_then(|pack| bpe::decode(&pack).ok());
    let (scene, mut textures, title, random, libc_random) = match forced_type {
        Some(_) => {
            let screen = LoadingScreen::new(game_random, libc_random, None, forced_type);
            let textures = load_screen_textures(&mut image, screen.screen_type())?;
            let title = format!(
                "Budokai Tenkaichi 3 - Loading Screen {}",
                screen.screen_type()
            );
            // The loading screen owns the shared generator here; the title is never reached.
            (
                Scene::Loading(screen),
                textures,
                title,
                GameRandom::from_key(&[0]),
                LibcRandom::new(0),
            )
        }
        None if skip_to_title => (
            Scene::Black,
            Vec::new(),
            "Budokai Tenkaichi 3".to_owned(),
            game_random,
            libc_random,
        ),
        None => (
            Scene::Logos(LogoSequence::new()),
            load_logo_textures(&mut image)?,
            "Budokai Tenkaichi 3".to_owned(),
            game_random,
            libc_random,
        ),
    };
    let boot = forced_type.is_none();
    let movie_texture = boot.then(|| {
        textures.push(RgbaTexture {
            width: movie::WIDTH,
            height: movie::HEIGHT,
            pixels: [0, 0, 0, 0xFF].repeat((movie::WIDTH * movie::HEIGHT) as usize),
        });
        textures.len() - 1
    });
    let font = load_fonts(&mut image, &mut textures)?;
    // BootCard_Init: the dialog of section 2 of menu archive 0 (its section 1), its textures filtered.
    let mut linear = Vec::new();
    let boot_dialog = if boot {
        match load_boot_dialog(&mut image) {
            Ok(assets) => {
                let indices = assets
                    .textures
                    .into_iter()
                    .map(|texture| {
                        texture.map(|texture| {
                            textures.push(texture.clone());
                            textures.push(texture);
                            linear.push(textures.len() - 2);
                            textures.len() - 2
                        })
                    })
                    .collect();
                dialog::Dialog::new(&assets.fod, assets.titles, indices, None, 0).ok()
            }
            Err(error) => {
                error!("cannot load the memory card check: {error}");
                None
            }
        }
    } else {
        None
    };
    let mut samplers = vec![Sampling::Nearest; textures.len()];
    for i in linear {
        samplers[i] = Sampling::Linear;
        samplers[i + 1] = Sampling::LinearRepeat;
    }
    let scene = match (scene, boot_dialog) {
        (Scene::Logos(_), Some(dialog)) => Scene::BootCard(Box::new(
            boot_card::BootCardScreen::new(dialog, card_res.as_deref()),
        )),
        (scene, _) => scene,
    };
    let soundtrack = boot.then(|| {
        let path = image_path.clone();
        std::thread::spawn(move || {
            let bytes = Iso9660::open(&path)
                .and_then(|mut iso| iso.read_file(movie::ADX_PATH))
                .map_err(|error| error.to_string())?;
            adx_to_wav(&bytes)
        })
    });
    let music = boot.then(|| decode_adx_file(image_path.clone(), title::MUSIC_FILE_ID));
    let engine = Arc::new(Mutex::new(se::Engine::default()));
    let mut effects = se::SoundEffects::new(engine.clone());
    // Snd_Init: the common bank (file 0x14B) goes to slot 0 at boot.
    effects.load_bank(
        1,
        se::LoadedBank::parse(&read_file(&mut image, COMMON_SE_BANK_FILE_ID)?)?,
    );

    let mut app = App::new();
    app.insert_resource(ClearColor(Color::BLACK))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title,
                        resolution: WindowResolution::new(SCREEN_WIDTH, SCREEN_HEIGHT)
                            .with_scale_factor_override(1.0),
                        resizable: false,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(Game {
            scene,
            textures,
            samplers,
            accumulator: 0.0,
            pressed_latch: 0,
            pressed_latch2: 0,
            battle_latch: [0; 2],
            quads: Vec::new(),
            iso_path: image_path,
            random,
            libc_random,
            vu0_random: rng::Vu0Random::default(),
            last_loading_type: None,
            model_textures: (Vec::new(), Vec::new()),
            model_rev: 0,
            model_batches: Vec::new(),
            stage_textures: Vec::new(),
            stage_rev: 0,
            stage_batches: Vec::new(),
            effect_textures: Vec::new(),
            effects: Vec::new(),
            under_effects: Vec::new(),
            shadow_effects: Vec::new(),
            rumble: [0; 2],
            rumble_sent: [0; 2],
            effect_point: Vec::new(),
            post_ops: Vec::new(),
            pre_ops: Vec::new(),
            depth_tint: None,
            effect_updates: Vec::new(),
            hud_textures: Vec::new(),
            hud_prims: Vec::new(),
            hud_layer: None,
            xfade_captures: 0,
            xfade_alpha: 0,
            hud_clamp_from: usize::MAX,
            demo: char_select::DemoProgress::default(),
            in_demo: false,
            story_battle: false,
            pending_script: None,
            ending_from: 6,
            story_ball: 0,
            movie_texture,
            soundtrack,
            soundtrack_source: None,
            music,
            music_source: None,
            title: None,
            main_menu: None,
            duel_menu: None,
            duel: DuelProgress::default(),
            hist_select: None,
            hist: hist_select::HistProgress::default(),
            mode_menu: None,
            sagas: HashMap::new(),
            hist_result: None,
            story_result,
            char_select: None,
            char_progress: CharSelProgress::default(),
            chr_table,
            pictures: HashMap::new(),
            save: new_save,
            card: memcard::McCard::new(memcard::Libmc::new(Some(memcard_dir))),
            progress: mcflow::Progress::default(),
            card_res,
            chara_table,
            dc_menu: None,
            options: None,
            char_ref: None,
            train: None,
            train_char_select: None,
            train_mode: None,
            shen: None,
            evo_top: None,
            evo_cursor: -1,
            evo_bgm: false,
            shop: None,
            evo_z: None,
            tour_menu: None,
            tour: tour_menu::TourSession::default(),
            shen_textures: false,
            chr_view: None,
            chr_view_rev: None,
            menu_sticks: [[0.0; 2]; 2],
            replay_menu: None,
            dc_pass: None,
            dc_list: None,
            dc: dc_menu::DcProgress::default(),
            last_battle: None,
            pending_replay: None,
            in_replay_list: false,
            bgm_sources: HashMap::new(),
            pending_bgm: None,
            save_loaded: false,
            main_menu_item: 0,
            bgm: None,
            map_bgm: None,
            pending_map: None,
            map_volume: 0x40,
            voice: [None; 2],
            stream_se: [None; 2],
            pending_stream_se: [None, None],
            se_voice_paused: false,
            voice_fresh: false,
            stream_fresh: false,
            stream_ended: false,
            voice_ended: false,
            pending_voice: [None, None],
            picture: None,
            skip_to_title,
            skip_to_menu,
            skip_to_battle,
            battle_stage,
            battle_mode,
            battle_split,
            se: effects,
            font,
        })
        .insert_resource(SeStream(engine))
        .add_audio_source::<se::SeOutput>()
        .init_resource::<Textures>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                run_frames,
                send_rumble.after(run_frames),
                sync_quads,
                model_draw::sync_models,
                model_draw::sync_stage,
                model_draw::sync_depth_tint,
                model_draw::sync_effects,
                model_draw::sync_hud,
                model_draw::sync_frame_buffers,
                post_draw::sync_post.after(model_draw::sync_frame_buffers),
                apply_volumes,
            )
                .chain(),
        );
    model_draw::add_to(&mut app);
    post_draw::add_to(&mut app);
    if let Some(path) = shot {
        app.insert_resource(Shot(path, shot_at))
            .add_systems(Update, take_shot);
    }
    if log_fps {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
    app.run();

    Ok(())
}

/// `--shot`: where the window is saved, and after how many updates (`--shot-at`, 600 by default).
#[derive(Resource)]
struct Shot(PathBuf, u32);

/// Saves the window after the shot's updates and ends the program 60 updates later.
fn take_shot(
    mut commands: Commands,
    shot: Res<Shot>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    *frames += 1;
    if *frames == shot.1 {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(shot.0.clone()));
    }
    if *frames == shot.1 + 60 {
        exit.write(AppExit::Success);
    }
}

fn load_screen_textures(
    image: &mut Iso9660,
    screen_type: u16,
) -> Result<Vec<RgbaTexture>, Box<dyn std::error::Error>> {
    let packed = read_file(image, FIRST_LOADING_FILE_ID + u32::from(screen_type))?;
    let decoded = bpe::decode(&packed)?;
    decode_textures(&decoded)
}

/// `FirstRun_Main`: section 3 of archive 0 is BPE-packed; its sections hold the six pictures.
/// `BootCard_Init`'s dialog: section 1 of the unpacked section 2 of menu archive 0.
fn load_boot_dialog(
    image: &mut Iso9660,
) -> Result<dialog::DialogAssets, Box<dyn std::error::Error>> {
    let archive = read_file(image, LOGO_ARCHIVE_FILE_ID)?;
    let res = bpe::decode(pack_section(&archive, boot_card::ARCHIVE_SECTION)?)?;
    dialog::DialogAssets::parse(pack_section(&res, boot_card::DIALOG_SECTION)?)
}

fn load_logo_textures(image: &mut Iso9660) -> Result<Vec<RgbaTexture>, Box<dyn std::error::Error>> {
    let archive = read_file(image, LOGO_ARCHIVE_FILE_ID)?;
    let pictures = bpe::decode(pack_section(&archive, 3)?)?;
    let mut textures = Vec::new();
    for section in logo::PICTURE_SECTIONS {
        let tiles = decode_textures(pack_section(&pictures, section)?)?;
        if tiles.len() != logo::TILES_PER_PICTURE {
            return Err(format!("logo section {section} has {} tiles", tiles.len()).into());
        }
        textures.extend(tiles);
    }
    Ok(textures)
}

fn read_file(image: &mut Iso9660, file_id: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let location = partition_entry_for_id(file_id);
    let archive = AfsArchive::open(image, format!("DATA/PZS3US{}.AFS", location.partition))?;
    Ok(archive.read_entry(image, location.index)?)
}

/// `Font_InitEx`: fonts 0 and 1 of the boot file (header words 1 and 2 are their byte offsets), one texture per
/// CLUT the glyphs use. GS sprites without TEX1 filtering sample point-wise.
fn load_fonts(
    image: &mut Iso9660,
    textures: &mut Vec<RgbaTexture>,
) -> Result<font::Font, Box<dyn std::error::Error>> {
    let boot = read_file(image, BOOT_FILE_ID)?;
    let word = |i: usize| -> Result<usize, Box<dyn std::error::Error>> {
        let bytes = boot
            .get(i * 4..i * 4 + 4)
            .ok_or("the boot file is too short")?;
        Ok(u32::from_le_bytes(bytes.try_into()?) as usize)
    };
    let mut slots = Vec::new();
    let mut line_height = 0;
    for slot in 1..=2 {
        let data = boot
            .get(word(slot)?..)
            .ok_or("font offset is outside the boot file")?;
        let file = FontFile::parse(data)?;
        let cluts = file.glyphs.iter().map(|g| g.clut() + 1).max().unwrap_or(1);
        let indices = decode_font_texture(&file.texture, cluts)?
            .into_iter()
            .map(|texture| {
                textures.push(texture);
                textures.len() - 1
            })
            .collect();
        slots.push(font::FontSlot::new(&file, indices));
        line_height = file.line_height;
    }
    let mut font = font::Font::new(slots, line_height);
    // FontIcon_Init: the icon file is at boot file word 8; the tag and icon tables are in the executable.
    let cnf = image.read_file("SYSTEM.CNF")?;
    let exe = bt3_data::elf::boot_path(&cnf).ok_or("SYSTEM.CNF names no executable")?;
    let elf = bt3_data::elf::Elf::parse(image.read_file(&exe)?)?;
    let mut icons = font::Icons::read(&elf)?;
    let res = bt3_data::tex::TextureResource::parse(
        boot.get(word(8)? & !3..)
            .ok_or("the icon file is outside the boot file")?,
    )?;
    for entry in &res.entries {
        let width = 1 << ((entry.texture_register >> 26) & 0xF);
        let texture = bt3_data::gs::decode_sprite_texture(entry).ok().map(|t| {
            textures.push(t);
            textures.len() - 1
        });
        icons.textures.push((texture, width));
    }
    font.icons = Some(icons);
    Ok(font)
}

/// `Title_Init`: section 1 of gMenuArc1, BPE-packed.
fn load_title_assets(iso_path: &Path) -> Result<TitleAssets, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(iso_path)?;
    let archive = read_file(&mut image, title::ARCHIVE_FILE_ID)?;
    let res = bpe::decode(pack_section(&archive, title::ARCHIVE_SECTION)?)?;
    TitleAssets::parse(&res)
}

fn adx_to_wav(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (header, mut pcm) = adx::decode(bytes).map_err(|error| error.to_string())?;
    // ADXT_SetOutputMono: a stereo stream played in mono has both channels mixed.
    if header.channels == 2 && sound::options().mono {
        for f in pcm.chunks_exact_mut(2) {
            let m = ((i32::from(f[0]) + i32::from(f[1])) / 2) as i16;
            f[0] = m;
            f[1] = m;
        }
    }
    Ok(adx::to_wav(
        header.sample_rate,
        header.channels as u16,
        &pcm,
    ))
}

/// An ADX file of the AFS archives, decoded to a WAV in the background.
fn decode_adx_file(iso_path: PathBuf, file_id: u32) -> Decoding {
    std::thread::spawn(move || {
        let mut image = Iso9660::open(&iso_path).map_err(|error| error.to_string())?;
        let bytes = read_file(&mut image, file_id).map_err(|error| error.to_string())?;
        adx_to_wav(&bytes)
    })
}

fn decode_textures(resource: &[u8]) -> Result<Vec<RgbaTexture>, Box<dyn std::error::Error>> {
    TextureResource::parse(resource)?
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn setup(mut commands: Commands, stream: Res<SeStream>, mut outputs: ResMut<Assets<se::SeOutput>>) {
    // The cameras are model_draw's: the 3D passes go to a frame buffer, the window shows it and the quads.
    let output = outputs.add(se::SeOutput(stream.0.clone()));
    commands.spawn(AudioPlayer(output));
}

/// The SPU2 the sound effects play on.
#[derive(Resource)]
struct SeStream(Arc<Mutex<se::Engine>>);

/// Pad 0 game buttons: arrows or the d-pad, Space or South (Cross), Escape or North (Triangle), Enter or Start,
/// C or East (Circle), Q / E or the shoulder buttons (L1 / R1), A / D or the triggers (L2 / R2).
const BUTTONS: [(KeyCode, GamepadButton, u32); 12] = [
    (KeyCode::ArrowLeft, GamepadButton::DPadLeft, title::PAD_LEFT),
    (
        KeyCode::ArrowRight,
        GamepadButton::DPadRight,
        title::PAD_RIGHT,
    ),
    (KeyCode::ArrowUp, GamepadButton::DPadUp, title::PAD_UP),
    (KeyCode::ArrowDown, GamepadButton::DPadDown, title::PAD_DOWN),
    (KeyCode::Space, GamepadButton::South, title::PAD_CROSS),
    (KeyCode::Escape, GamepadButton::North, title::PAD_TRIANGLE),
    (KeyCode::Enter, GamepadButton::Start, title::PAD_START),
    (KeyCode::KeyC, GamepadButton::East, dc_password::PAD_CIRCLE),
    (
        KeyCode::KeyQ,
        GamepadButton::LeftTrigger,
        dc_password::PAD_L1,
    ),
    (
        KeyCode::KeyE,
        GamepadButton::RightTrigger,
        dc_password::PAD_R1,
    ),
    (
        KeyCode::KeyA,
        GamepadButton::LeftTrigger2,
        dc_password::PAD_L2,
    ),
    (
        KeyCode::KeyD,
        GamepadButton::RightTrigger2,
        dc_password::PAD_R2,
    ),
];

/// Pad 0's `PAD_*` bits in battle: the d-pad on the arrows; Z, X, C, V for Square, Cross, Circle, Triangle (Space is
/// Cross as in the menus); Q / E for L1 / R1, A / D for L2 / R2; Enter for Start.
const BATTLE_KEYS: [(KeyCode, u32); 14] = [
    (KeyCode::ArrowUp, btl_input::pad::UP),
    (KeyCode::ArrowDown, btl_input::pad::DOWN),
    (KeyCode::ArrowLeft, btl_input::pad::LEFT),
    (KeyCode::ArrowRight, btl_input::pad::RIGHT),
    (KeyCode::KeyZ, btl_input::pad::SQUARE),
    (KeyCode::KeyX, btl_input::pad::CROSS),
    (KeyCode::Space, btl_input::pad::CROSS),
    (KeyCode::KeyC, btl_input::pad::CIRCLE),
    (KeyCode::KeyV, btl_input::pad::TRIANGLE),
    (KeyCode::KeyQ, btl_input::pad::L1),
    (KeyCode::KeyE, btl_input::pad::R1),
    (KeyCode::KeyA, btl_input::pad::L2),
    (KeyCode::KeyD, btl_input::pad::R2),
    (KeyCode::Enter, btl_input::pad::START),
];

/// The same from a gamepad.
const BATTLE_BUTTONS: [(GamepadButton, u32); 16] = [
    (GamepadButton::DPadUp, btl_input::pad::UP),
    (GamepadButton::DPadDown, btl_input::pad::DOWN),
    (GamepadButton::DPadLeft, btl_input::pad::LEFT),
    (GamepadButton::DPadRight, btl_input::pad::RIGHT),
    (GamepadButton::West, btl_input::pad::SQUARE),
    (GamepadButton::South, btl_input::pad::CROSS),
    (GamepadButton::East, btl_input::pad::CIRCLE),
    (GamepadButton::North, btl_input::pad::TRIANGLE),
    (GamepadButton::LeftTrigger, btl_input::pad::L1),
    (GamepadButton::RightTrigger, btl_input::pad::R1),
    (GamepadButton::LeftTrigger2, btl_input::pad::L2),
    (GamepadButton::RightTrigger2, btl_input::pad::R2),
    (GamepadButton::Select, btl_input::pad::SELECT),
    (GamepadButton::Start, btl_input::pad::START),
    (GamepadButton::LeftThumb, btl_input::pad::L3),
    (GamepadButton::RightThumb, btl_input::pad::R3),
];

/// `Voice_GetStat(0)`: 2 while the file is decoded, 3 while it sounds, 5 once it played to its end, else 0.
fn voice_stat(game: &mut Game, players: &Query<(), With<AudioPlayer>>) -> i32 {
    if game.pending_voice[0].is_some() {
        return 2;
    }
    match game.voice[0] {
        Some(player) if game.voice_fresh || players.contains(player.entity) => 3,
        Some(_) => {
            game.voice[0] = None;
            game.voice_ended = true;
            anim::VOICE_PLAYEND
        }
        None if game.voice_ended => anim::VOICE_PLAYEND,
        None => 0,
    }
}

/// `Voice_GetStat(1)`: 2 while the file is decoded, 3 while it sounds, else 0.
fn voice1_stat(game: &Game, players: &Query<(), With<AudioPlayer>>) -> i32 {
    if game.pending_voice[1].is_some() {
        2
    } else if game.voice[1].is_some_and(|p| players.contains(p.entity)) {
        3
    } else {
        0
    }
}

/// `StreamSe_GetStat(0)`, as `voice_stat`.
fn stream_se_stat(game: &mut Game, players: &Query<(), With<AudioPlayer>>) -> i32 {
    if game.pending_stream_se[0].is_some() {
        return 2;
    }
    match game.stream_se[0] {
        Some(player) if game.stream_fresh || players.contains(player.entity) => 3,
        Some(_) => {
            game.stream_se[0] = None;
            game.stream_ended = true;
            anim::VOICE_PLAYEND
        }
        None if game.stream_ended => anim::VOICE_PLAYEND,
        None => 0,
    }
}

/// `Adx_StopAll`: `Adx_ResumeSeVoice`, then every player stops.
fn stop_all(game: &mut Game, commands: &mut Commands) {
    game.se_voice_paused = false;
    stop_player(&mut game.bgm, commands);
    stop_player(&mut game.map_bgm, commands);
    game.pending_map = None;
    for i in 0..2 {
        stop_player(&mut game.voice[i], commands);
        stop_player(&mut game.stream_se[i], commands);
        game.pending_voice[i] = None;
        game.pending_stream_se[i] = None;
    }
    game.pending_bgm = None;
    game.voice_ended = false;
    game.stream_ended = false;
}

/// What a menu frame asked of the sound players.
fn play_sounds(game: &mut Game, commands: &mut Commands, sounds: Vec<TitleSound>) {
    for sound in sounds {
        match sound {
            TitleSound::PauseSeVoice(on) => game.se_voice_paused = on,
            TitleSound::Voice(id) => play_voice(game, commands, 0, id, 0x80),
            TitleSound::VoiceEx {
                player, id, volume, ..
            } => play_voice(game, commands, player as usize & 1, id, volume),
            TitleSound::VoiceStop => {
                stop_player(&mut game.voice[0], commands);
                game.pending_voice[0] = None;
                game.voice_ended = false;
            }
            TitleSound::VoiceStopEx(player) => {
                let i = player as usize & 1;
                stop_player(&mut game.voice[i], commands);
                game.pending_voice[i] = None;
                if i == 0 {
                    game.voice_ended = false;
                }
            }
            TitleSound::FadeOutStep => {
                for player in [&mut game.voice[0], &mut game.bgm].into_iter().flatten() {
                    player.level = sound::fade_out_step(player.level);
                }
            }
            TitleSound::Bgm(file_id) => {
                stop_player(&mut game.bgm, commands);
                game.pending_bgm = None;
                play_bgm(game, commands, file_id);
            }
            TitleSound::VoiceFadeOutStep => {
                if let Some(player) = &mut game.voice[0] {
                    player.level = sound::fade_out_step(player.level);
                }
            }
            TitleSound::BgmStop => {
                stop_player(&mut game.bgm, commands);
                game.pending_bgm = None;
            }
            TitleSound::BgmFadeOutStep => {
                if let Some(player) = &mut game.bgm {
                    player.level = sound::fade_out_step(player.level);
                }
            }
            TitleSound::Effect(bank, id) => game.se.play_se(bank, id),
            TitleSound::SeEx {
                mask,
                id,
                volume,
                pan,
                pitch,
                handle,
            } => game.se.play_se_ex(mask, id, volume, pan, pitch, handle),
            TitleSound::SeStop(handle) => game.se.stop_handle(handle),
            TitleSound::SeStopBanks(mask) => game.se.stop_banks(mask),
            TitleSound::MapBgm(id) => {
                stop_player(&mut game.map_bgm, commands);
                game.map_volume = 0x40;
                game.pending_map = Some(decode_adx_file(game.iso_path.clone(), id));
            }
            TitleSound::MapBgmVolume(volume) => {
                game.map_volume = volume;
                if let Some(p) = &mut game.map_bgm {
                    p.level = sound::out_volume(sound::CH_MAP, volume);
                }
            }
            TitleSound::MapBgmStop => {
                stop_player(&mut game.map_bgm, commands);
                game.pending_map = None;
            }
            TitleSound::StreamSe(id) => play_stream_se(game, commands, 0, id, 0x40),
            TitleSound::StreamSeFadeOutStep(player) => {
                if let Some(p) = &mut game.stream_se[player as usize & 1] {
                    p.level = sound::fade_out_step(p.level);
                }
            }
            TitleSound::StreamSeEx {
                player, id, volume, ..
            } => play_stream_se(game, commands, player as usize & 1, id, volume),
        }
    }
}

/// `Voice_Play(player, id, volume, pan)`: the player restarts with the file once it is decoded. The pan is not
/// applied (Bevy's players have none).
fn play_voice(game: &mut Game, commands: &mut Commands, player: usize, id: u32, volume: i32) {
    stop_player(&mut game.voice[player], commands);
    if player == 0 {
        game.voice_ended = false;
    }
    game.pending_voice[player] = Some((decode_adx_file(game.iso_path.clone(), id), volume));
}

/// `StreamSe_Play(player, id, volume, pan)`, as `play_voice`.
fn play_stream_se(game: &mut Game, commands: &mut Commands, player: usize, id: u32, volume: i32) {
    stop_player(&mut game.stream_se[player], commands);
    game.pending_stream_se[player] = Some((decode_adx_file(game.iso_path.clone(), id), volume));
}

/// `Pad_SendActuator` of pad 0 (every connected gamepad, as pad 0 reads them all): the last battle frame's motors,
/// held for one 30 Hz frame and replaced by the next request.
fn send_rumble(
    gamepads: Query<Entity, With<Gamepad>>,
    mut game: ResMut<Game>,
    mut rumble: MessageWriter<bevy::input::gamepad::GamepadRumbleRequest>,
) {
    // Each port's request to its controller (`ports`: the first gamepad is port 1, the others port 2).
    let mut ports: Vec<Entity> = gamepads.iter().collect();
    ports.sort();
    for (i, gamepad) in ports.into_iter().enumerate() {
        let port = i.min(1);
        let data = game.rumble[port];
        if data == game.rumble_sent[port] && data == 0 {
            continue;
        }
        rumble.write(bevy::input::gamepad::GamepadRumbleRequest::Stop { gamepad });
        if data != 0 {
            rumble.write(bevy::input::gamepad::GamepadRumbleRequest::Add {
                gamepad,
                duration: std::time::Duration::from_millis(50),
                intensity: bevy::input::gamepad::GamepadRumbleIntensity {
                    strong_motor: f32::from(data >> 1) / 127.0,
                    weak_motor: if data & 1 != 0 { 1.0 } else { 0.0 },
                },
            });
        }
    }
    game.rumble_sent = game.rumble;
}

fn run_frames(
    mut commands: Commands,
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    gamepad_query: Query<(Entity, &Gamepad)>,
    players: Query<(), With<AudioPlayer>>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut game: ResMut<Game>,
) {
    // The controller ports: the keyboard and the first gamepad are port 1, the other gamepads port 2.
    let mut sorted: Vec<(Entity, &Gamepad)> = gamepad_query.iter().collect();
    sorted.sort_by_key(|(e, _)| *e);
    let port_of = |i: usize| i.min(1);
    // Each port for the battle: `Pad.held` with a latch that keeps a press shorter than a frame for one frame,
    // and the left stick as `Pad_Update` reads it (y down).
    let mut battle_held = [0u32; 2];
    for (key, bit) in BATTLE_KEYS {
        if keyboard.just_pressed(key) {
            game.battle_latch[0] |= bit;
        }
        if keyboard.pressed(key) {
            battle_held[0] |= bit;
        }
    }
    let mut battle_stick = [[0.0f32; 2]; 2];
    let mut menu_sticks = [[0.0f32; 2]; 2];
    for (i, (_, pad)) in sorted.iter().enumerate() {
        let port = port_of(i);
        for (button, bit) in BATTLE_BUTTONS {
            if pad.just_pressed(button) {
                game.battle_latch[port] |= bit;
            }
            if pad.pressed(button) {
                battle_held[port] |= bit;
            }
        }
        if let (Some(x), Some(y)) = (
            pad.get(GamepadAxis::LeftStickX),
            pad.get(GamepadAxis::LeftStickY),
        ) {
            let raw = |v: f32| (v * 127.5 + 127.5).clamp(0.0, 255.0) as u8;
            let s = btl_input::stick_from_raw(raw(x), raw(-y));
            if s != [0.0; 2] {
                battle_stick[port] = s;
            }
            if port == 0 {
                menu_sticks[0] = s;
                let rx = pad.get(GamepadAxis::RightStickX).unwrap_or(0.0);
                let ry = pad.get(GamepadAxis::RightStickY).unwrap_or(0.0);
                menu_sticks[1] = [
                    btl_input::stick_axis(raw(rx)),
                    btl_input::stick_axis(raw(-ry)),
                ];
            }
        }
    }
    game.menu_sticks = menu_sticks;
    let mut held = 0;
    let mut held2 = 0;
    for (key, button, bit) in BUTTONS {
        let port1 = sorted.iter().take(1);
        let port2 = sorted.iter().skip(1);
        if keyboard.just_pressed(key) || port1.clone().any(|(_, pad)| pad.just_pressed(button)) {
            game.pressed_latch |= bit;
        }
        if keyboard.pressed(key) || port1.clone().any(|(_, pad)| pad.pressed(button)) {
            held |= bit;
        }
        if port2.clone().any(|(_, pad)| pad.just_pressed(button)) {
            game.pressed_latch2 |= bit;
        }
        if port2.clone().any(|(_, pad)| pad.pressed(button)) {
            held2 |= bit;
        }
    }
    // The loaded save's sound options are the ones in force; when they change, SndOpt_Apply's
    // Bgm_SetVolume(0x40) takes the music to its new level.
    if sound::set_options(game.save.sound_options())
        && let Some(player) = &mut game.bgm
    {
        player.level = sound::out_volume(sound::CH_BGM, 0x40);
    }
    game.voice_fresh = false;
    game.stream_fresh = false;
    start_pending_voice(&mut game, &mut commands, &mut audio);
    start_pending_stream_se(&mut game, &mut commands, &mut audio);
    start_pending_map(&mut game, &mut commands, &mut audio);
    start_pending_bgm(&mut game, &mut commands, &mut audio);
    let pad2 = sorted.len() >= 2;
    if std::mem::take(&mut game.skip_to_title) {
        if game.skip_to_battle {
            match quick_battle_setup(&mut game) {
                Ok(setup) => start_battle(&mut game, setup),
                Err(error) => {
                    error!("cannot set up the battle: {error}");
                    game.scene = Scene::Black;
                }
            }
        } else if let Some(won) = game.story_result {
            load_menu_bank(&mut game);
            game.save_loaded = true;
            start_hist_result(&mut game, won, 0);
        } else if game.skip_to_menu {
            load_menu_bank(&mut game);
            game.save_loaded = true;
            start_main_menu(&mut game, &mut commands, &mut audio);
        } else {
            start_title(&mut game, &mut commands, &mut audio);
        }
    }
    // A frame that runs late slows the game down like the PS2's would instead of piling up catch-up frames.
    let frame_seconds = game.scene.frame_seconds();
    game.accumulator = (game.accumulator + time.delta_secs()).min(MAX_CATCH_UP * frame_seconds);
    while game.accumulator >= game.scene.frame_seconds() {
        game.accumulator -= game.scene.frame_seconds();
        // Only the last frame of this update is presented.
        let shown = game.accumulator < game.scene.frame_seconds();
        let pressed = std::mem::take(&mut game.pressed_latch);
        let pressed2 = std::mem::take(&mut game.pressed_latch2);
        let cross = pressed & title::PAD_CROSS != 0;
        let start = pressed & title::PAD_START != 0;
        // Snd_Update at the top of every frame loop sends what the last frame queued.
        game.se.update();
        let game = &mut *game;
        let stat = voice_stat(game, &players);
        let stream_stat = stream_se_stat(game, &players);
        // The streams a story battle's script waits on.
        let battle_streams = btl_script::StreamStat {
            voice: [stat, voice1_stat(game, &players)],
            bgm_playing: game.pending_bgm.is_some()
                || game.bgm.is_some_and(|p| players.contains(p.entity)),
        };
        // Gfx_EndFrame(vsyncs) of the screen this frame runs.
        let vsyncs = if game.scene.frame_seconds() > VBLANK_SECONDS {
            2
        } else {
            1
        };
        match &mut game.scene {
            Scene::BootCard(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(held, pressed, &mut game.font, &mut mc);
                let done = screen.is_done();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    game.scene = Scene::Logos(LogoSequence::new());
                }
            }
            Scene::EvoTop(screen) => {
                let dirty = game.dc.dirty;
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    dirty,
                    &mut game.random,
                    &mut game.font,
                    &mut mc,
                );
                let result = screen.result();
                let (cursor, save_started) = (screen.cursor(), screen.save_started());
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // EvoTop_Leave: the cursor kept; DcSave_Term: a started save flips the flag back.
                    game.evo_cursor = cursor;
                    if save_started {
                        game.dc.dirty = !game.dc.dirty;
                    }
                    match result {
                        0 => {
                            // EvoMode_Main: Adx_StopAll, back to the main menu.
                            game.evo_bgm = false;
                            stop_all(game, &mut commands);
                            start_main_menu(game, &mut commands, &mut audio);
                        }
                        1 => start_evo_z(game),
                        _ => start_shop(game),
                    }
                }
            }
            Scene::TourMenu(screen) => {
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                let session = screen.session();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // Tour_Main: Adx_StopAll after TourMenu_Run.
                    stop_all(game, &mut commands);
                    if result != 0 {
                        game.tour = session;
                        info!("the tournament's entrant select (mode 34) is not ported yet");
                        start_tour_menu(game, &mut commands);
                    } else {
                        start_main_menu(game, &mut commands, &mut audio);
                    }
                }
            }
            Scene::EvoZ(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    &mut game.save,
                    &mut loader,
                    &mut game.libc_random,
                    &mut game.font,
                );
                let done = screen.result().is_some();
                // gProgress->flags |= 1 when the save changed.
                if screen.dirty() {
                    game.dc.dirty = true;
                }
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // EvoMode_Main: back to mode 48, the music going on.
                    start_evo_top(game, &mut commands);
                }
            }
            Scene::Shop(screen) => {
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.save,
                    &game.chr_table,
                    &mut game.random,
                    &mut game.font,
                );
                let done = screen.result().is_some();
                // gProgress->flags |= 1 on a purchase.
                if screen.dirty() {
                    game.dc.dirty = true;
                }
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // EvoMode_Main: back to mode 48, the music going on.
                    start_evo_top(game, &mut commands);
                }
            }
            Scene::Shen(screen) => {
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.save,
                    &game.chr_table,
                    &mut game.random,
                    &mut game.libc_random,
                    &mut game.vu0_random,
                    &mut game.font,
                );
                // The backdrop's stage and models, as the model viewer hands them over.
                if !game.shen_textures {
                    game.shen_textures = true;
                    game.stage_textures = screen.scene().stage_textures.clone();
                    game.stage_rev += 1;
                    game.model_textures = screen.scene().model_textures();
                    game.model_rev += 1;
                }
                let done = screen.is_done();
                game.stage_batches = frame.scene.stage;
                game.model_batches = frame.scene.models;
                game.post_ops = frame.scene.post;
                if frame.scene.capture {
                    game.xfade_captures += 1;
                }
                game.xfade_alpha = frame.scene.xfade_alpha;
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // Shen_Main: Adx_StopAll, then mode 71 (ShenScene_Term ends the backdrop).
                    game.shen_textures = false;
                    game.stage_batches.clear();
                    game.model_batches.clear();
                    game.post_ops.clear();
                    game.xfade_alpha = 0;
                    stop_all(game, &mut commands);
                    start_shen_save(game);
                }
            }
            Scene::ShenSave(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(held, pressed, &mut game.font, &mut mc);
                let done = screen.is_done();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // Shen_Main: Adx_StopAll, mode 4.
                    stop_all(game, &mut commands);
                    start_main_menu(game, &mut commands, &mut audio);
                }
            }
            Scene::Training(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.save,
                    &mut loader,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.progress.train = screen.progress();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // TrainMode_Main: Adx_StopAll after Train_Run.
                    stop_all(game, &mut commands);
                    match result {
                        training::TrainResult::Back => {
                            start_main_menu(game, &mut commands, &mut audio)
                        }
                        training::TrainResult::Select => start_train_select(game, &mut commands),
                        training::TrainResult::Lesson(Some(setup)) => {
                            game.train_mode = Some(44);
                            start_battle(game, setup);
                        }
                        training::TrainResult::Lesson(None) => {
                            // Only the lessons with no listed page set no battle.
                            info!("the lesson sets up no battle");
                            start_training(game, &mut commands);
                        }
                    }
                }
            }
            Scene::CharRef(screen) => {
                // CharRef_Draw's Job_Run (the viewer's load job) runs unless the viewer has the screen.
                let viewer_shown = screen.viewer_shown();
                if !viewer_shown && let Some(viewer) = &mut game.chr_view {
                    let iso_path = game.iso_path.clone();
                    let mut read = |id: u32| {
                        Iso9660::open(&iso_path)
                            .map_err(|e| e.to_string())
                            .and_then(|mut image| {
                                read_file(&mut image, id).map_err(|e| e.to_string())
                            })
                    };
                    viewer.step_job(&mut read);
                }
                let viewer_visible = game.chr_view.as_ref().is_some_and(|v| v.is_visible());
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    viewer_visible,
                    &mut loader,
                    &mut game.random,
                    &mut game.libc_random,
                    &mut game.font,
                );
                let done = screen.is_done();
                let requests = screen.take_viewer_requests();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                // ChrView_Update while the viewer has the screen: the backdrop and the model, nothing else.
                game.stage_batches.clear();
                game.model_batches.clear();
                if viewer_shown && let Some(viewer) = &mut game.chr_view {
                    let [left, right] = game.menu_sticks;
                    let out = viewer.update(left, right[1], &mut game.libc_random);
                    if game.chr_view_rev != Some(viewer.model_rev) {
                        game.chr_view_rev = Some(viewer.model_rev);
                        game.model_textures = viewer.model_textures();
                        game.model_rev += 1;
                        game.stage_textures = viewer.stage_textures.clone();
                        game.stage_rev += 1;
                    }
                    game.stage_batches = out.stage;
                    game.model_batches = out.models;
                }
                if let Some(viewer) = &mut game.chr_view {
                    for request in requests {
                        match request {
                            char_ref::ViewerRequest::Show { chara, costume } => {
                                viewer.show(chara, costume, false)
                            }
                            char_ref::ViewerRequest::Hide => viewer.hide(),
                        }
                    }
                }
                if done {
                    // CharRefMode_Main: Adx_StopAll, back to the main menu (CharRef_Term ends the viewer).
                    game.chr_view = None;
                    game.chr_view_rev = None;
                    stop_all(game, &mut commands);
                    start_main_menu(game, &mut commands, &mut audio);
                }
            }
            Scene::Options(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.random,
                    &mut game.font,
                    &mut mc,
                );
                let done = screen.is_done();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // OptMode_Main: Adx_StopAll, back to the main menu.
                    stop_all(game, &mut commands);
                    start_main_menu(game, &mut commands, &mut audio);
                }
            }
            Scene::DcMenu(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.dc,
                    &mut game.random,
                    &mut game.font,
                    &mut mc,
                );
                let result = screen.result();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(next) = result {
                    stop_all(game, &mut commands);
                    match next {
                        54 => start_dc_pass(game, &mut commands),
                        55 => start_dc_list(game, &mut commands),
                        56 => start_replay_menu(game, &mut commands),
                        0 => {
                            game.dc.visits += 1;
                            start_main_menu(game, &mut commands, &mut audio);
                        }
                        mode => {
                            info!("progress mode {mode} is not a Data Center mode");
                            start_dc_menu(game, &mut commands);
                        }
                    }
                }
            }
            Scene::DcPass(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.dc,
                    &mut game.save,
                    &mut game.random,
                    &mut loader,
                    &mut game.font,
                );
                let done = screen.is_done();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    // Dc_Main: back to the top menu.
                    stop_all(game, &mut commands);
                    start_dc_menu(game, &mut commands);
                }
            }
            Scene::DcList(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.dc,
                    &mut game.save,
                    &game.chr_table,
                    &mut game.random,
                    &mut loader,
                    &mut game.font,
                );
                let done = screen.is_done();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if done {
                    stop_all(game, &mut commands);
                    start_dc_menu(game, &mut commands);
                }
            }
            Scene::ReplayMenu(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(held, pressed, &mut game.font, &mut mc);
                let result = screen.result();
                let cursor = screen.cursor();
                let loaded = result.and_then(|_| screen.take_loaded());
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    game.dc.replay_cursor = cursor;
                    stop_all(game, &mut commands);
                    match (result, loaded) {
                        (replay_menu::ReplayMenuResult::Play, Some((setup, replay))) => {
                            // Dc_Main leaves for the battle with the loaded replay.
                            game.dc.visits += 1;
                            game.in_replay_list = true;
                            game.pending_replay = Some(replay);
                            start_battle(game, setup);
                        }
                        (replay_menu::ReplayMenuResult::Select(39), _) => {
                            game.dc.visits += 1;
                            start_char_select(game, &mut commands);
                        }
                        (replay_menu::ReplayMenuResult::Select(_), _) => {
                            game.dc.visits += 1;
                            start_team_select(game, &mut commands);
                        }
                        _ => start_dc_menu(game, &mut commands),
                    }
                }
            }
            Scene::Logos(logos) => {
                game.quads = into_screen(logos.run_frame(cross || start));
                if logos.is_finished() {
                    start_opening_movie(game, &mut commands, &mut audio);
                }
            }
            Scene::Movie(player, video, ending) => {
                let ending = *ending;
                let frame = player.run_frame(start, || video.next_picture());
                game.quads = into_screen(frame.quads);
                if frame.picture.is_some() {
                    game.picture = frame.picture;
                }
                if player.is_finished() {
                    // The soundtrack plays on the BGM player.
                    stop_player(&mut game.bgm, &mut commands);
                    if ending {
                        finish_ending_movie(game, &mut commands);
                    } else {
                        start_title(game, &mut commands, &mut audio);
                    }
                }
            }
            Scene::Title(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    &mut game.demo,
                    &mut game.random,
                    &mut game.font,
                    &mut mc,
                );
                let result = screen.result();
                // The setup Title_Idle made when the screen began to leave.
                let demo = result.and_then(|_| screen.take_demo());
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // Progress_Main: Adx_StopAll after Title_Run.
                    stop_all(game, &mut commands);
                    match result {
                        TitleResult::OpeningMovie => {
                            start_opening_movie(game, &mut commands, &mut audio)
                        }
                        TitleResult::MainMenu => {
                            game.save_loaded = true;
                            start_main_menu(game, &mut commands, &mut audio);
                        }
                        TitleResult::Demo => match demo {
                            Some(setup) => {
                                game.in_demo = true;
                                start_battle(game, setup);
                            }
                            None => game.scene = Scene::Black,
                        },
                    }
                }
            }
            Scene::MainMenu(screen) => {
                let frame = screen.run_frame(held, pressed, stat, &mut game.random, &mut game.font);
                let result = screen.result();
                let item = screen.menu_item();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    game.main_menu_item = item;
                    stop_all(game, &mut commands);
                    match result {
                        MainMenuResult::Title => start_title(game, &mut commands, &mut audio),
                        MainMenuResult::Mode(6) => start_hist_select(game, &mut commands),
                        MainMenuResult::Mode(38) => start_duel_menu(game, &mut commands),
                        MainMenuResult::Mode(53) => {
                            // Dc_Main: the top menu's and the list's cursors start over.
                            game.dc.cursor = -1;
                            game.dc.replay_cursor = 0;
                            start_dc_menu(game, &mut commands);
                        }
                        MainMenuResult::Mode(62) => start_options(game, &mut commands),
                        MainMenuResult::Mode(60) => start_char_ref(game, &mut commands),
                        MainMenuResult::Mode(44) => start_training(game, &mut commands),
                        MainMenuResult::Mode(70) => start_shen(game, &mut commands),
                        MainMenuResult::Mode(33) => start_tour_menu(game, &mut commands),
                        MainMenuResult::Mode(48) => {
                            // EvoMode_Main: the cursor starts over.
                            game.evo_cursor = -1;
                            start_evo_top(game, &mut commands);
                        }
                        MainMenuResult::Mode(mode) => {
                            info!("progress mode {mode} is not ported yet");
                            game.scene = Scene::Black;
                        }
                    }
                }
            }
            Scene::DuelMenu(screen) => {
                let frame =
                    screen.run_frame(held, pressed, pad2, stat, &mut game.random, &mut game.font);
                let result = screen.result();
                game.duel = screen.progress();
                game.save.rules = screen.rules();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    stop_all(game, &mut commands);
                    match result {
                        DuelMenuResult::Back => start_main_menu(game, &mut commands, &mut audio),
                        DuelMenuResult::Select(39) => start_char_select(game, &mut commands),
                        DuelMenuResult::Select(_) => start_team_select(game, &mut commands),
                    }
                }
            }
            Scene::CharSelect(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    [(pressed, held), (pressed2, held2)],
                    &mut loader,
                    &game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.char_progress = screen.progress();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // Duel_Main / TrainMode_Main: Adx_StopAll after CharSel_Run.
                    stop_all(game, &mut commands);
                    match (result, game.train_mode) {
                        // TrainMode_Main mode 45: back to the training menu.
                        (CharSelResult::Back, Some(_)) => {
                            game.train_mode = None;
                            start_training(game, &mut commands);
                        }
                        (CharSelResult::Back, None) => start_duel_menu(game, &mut commands),
                        (CharSelResult::Battle(mut setup), Some(_)) => {
                            setup.tutorial = game.progress.train.tutorial;
                            start_battle(game, setup);
                        }
                        (CharSelResult::Battle(setup), None) => start_battle(game, setup),
                    }
                }
            }
            Scene::HistSelect(screen) => {
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.hist = screen.progress();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // Hist_Main: Adx_StopAll after HistSel_Run.
                    stop_all(game, &mut commands);
                    match result {
                        hist_select::HistSelResult::Back => {
                            start_main_menu(game, &mut commands, &mut audio)
                        }
                        hist_select::HistSelResult::Saga => start_mode_menu(game, &mut commands, 6),
                        hist_select::HistSelResult::Ending => {
                            start_ending_movie(game, &mut commands, &mut audio, 6)
                        }
                    }
                }
            }
            Scene::ModeMenu(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    held,
                    pressed,
                    stat,
                    &mut loader,
                    &mut game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.hist.sub_menu_item = screen.item();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    stop_all(game, &mut commands);
                    match result {
                        mode_menu::ModeMenuResult::Back => start_hist_select(game, &mut commands),
                        mode_menu::ModeMenuResult::Battle(script) => {
                            start_story_battle(game, script)
                        }
                    }
                }
            }
            Scene::HistResult(screen) => {
                let frame = screen.run_frame(pressed, stream_stat, &mut game.save, &mut game.font);
                let result = screen.result();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    stop_all(game, &mut commands);
                    match result {
                        hist_result::HistResultResult::Lost => {
                            start_mode_menu(game, &mut commands, 8)
                        }
                        hist_result::HistResultResult::Won => start_hist_save(game, 8),
                        hist_result::HistResultResult::Cleared => {
                            start_hist_outro(game, &mut commands)
                        }
                    }
                }
            }
            Scene::HistOutro(screen) => {
                let frame = screen.run_frame(
                    pressed,
                    stat,
                    &mut game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    stop_all(game, &mut commands);
                    match result {
                        hist_outro::HistOutroResult::Save => start_hist_save(game, 9),
                        hist_outro::HistOutroResult::Ending => {
                            start_ending_movie(game, &mut commands, &mut audio, 9)
                        }
                    }
                }
            }
            Scene::HistSave(screen) => {
                let mut mc = mcflow::McEnv {
                    card: &mut game.card,
                    save: &mut game.save,
                    progress: &mut game.progress,
                    items: &game.chr_table.items,
                };
                let frame = screen.run_frame(held, pressed, &mut game.font, &mut mc);
                let result = screen.result();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    stop_all(game, &mut commands);
                    match result {
                        hist_save::HistSaveResult::Sagas => start_hist_select(game, &mut commands),
                        hist_save::HistSaveResult::Episodes => {
                            start_mode_menu(game, &mut commands, 10)
                        }
                    }
                }
            }
            Scene::TeamSelect(screen) => {
                let mut loader = Pictures {
                    iso_path: &game.iso_path,
                    textures: &mut game.textures,
                    samplers: &mut game.samplers,
                    cache: &mut game.pictures,
                };
                let frame = screen.run_frame(
                    [(pressed, held), (pressed2, held2)],
                    &mut loader,
                    &game.save,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
                game.char_progress = screen.progress();
                game.quads = frame.quads;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(result) = result {
                    // Duel_Main: Adx_StopAll after TeamSel_Run.
                    stop_all(game, &mut commands);
                    match result {
                        CharSelResult::Back => start_duel_menu(game, &mut commands),
                        CharSelResult::Battle(setup) => start_battle(game, setup),
                    }
                }
            }
            Scene::Black => game.quads.clear(),
            Scene::Loading(screen) => game.quads = into_screen(screen.run_frame([cross, false])),
            Scene::BattleLoad(load) => {
                // Battle_Load: Load_RunBlocking runs the jobs (here the reader thread) under the loading screen.
                let busy = load.job.as_ref().is_some_and(|job| !job.is_finished());
                let quads = load.screen.run_job_frame([cross, false], busy);
                game.quads = into_screen_at(quads, load.texture_base);
                if load.screen.is_done() {
                    finish_battle_load(game, &mut commands);
                }
            }
            Scene::Battle(battle) => {
                let pads: [btl_input::PadState; 2] =
                    std::array::from_fn(|port| btl_input::PadState {
                        held: battle_held[port] | std::mem::take(&mut game.battle_latch[port]),
                        left: battle_stick[port],
                        status: 0,
                    });
                battle.stream_stat = battle_streams;
                let winner = battle.seq.winner;
                game.story_ball = battle
                    .assets
                    .hidden_item
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .found;
                let frame = battle.run_frame(
                    start,
                    pads,
                    stat,
                    &mut game.libc_random,
                    &mut game.random,
                    Some(&mut game.font),
                    shown,
                );
                // The textures a character change added (the draw uploads what is new).
                let a = &battle.assets;
                for (have, all) in [
                    (&mut game.model_textures.0, &a.base_textures),
                    (&mut game.model_textures.1, &a.shade_textures),
                    (&mut game.effect_textures, &a.effect_textures),
                ] {
                    if have.len() < all.len() {
                        have.extend_from_slice(&all[have.len()..]);
                    }
                }
                // The banks the battle loads (Snd_LoadBank / Snd_ReloadBank).
                for (mask, file) in std::mem::take(&mut battle.assets.bank_requests) {
                    match file.as_deref().map(se::LoadedBank::parse) {
                        Some(Ok(bank)) => game.se.reload_bank(mask, bank),
                        Some(Err(e)) => error!("sample bank {mask:#X}: {e}"),
                        None => game.se.unload_bank(mask),
                    }
                }
                let a = &battle.assets;
                // A stage change's new stage textures.
                if a.stage_rev != game.stage_rev {
                    game.stage_textures = a.stage_textures.clone();
                    game.stage_rev = a.stage_rev;
                }
                game.model_batches = frame.models;
                game.stage_batches = frame.stage;
                game.effects = frame.effects;
                game.under_effects = frame.under;
                game.shadow_effects = frame.shadows;
                game.post_ops = frame.post;
                game.pre_ops = frame.pre;
                game.depth_tint = frame.depth_tint;
                game.effect_updates.clear();
                for (i, t) in frame.texture_updates {
                    if let Some(slot) = game.effect_textures.get_mut(i) {
                        *slot = t;
                        game.effect_updates.push(i);
                    }
                }
                // The menu texts, then the screen fade over everything.
                let mut quads = Vec::new();
                game.font.flush(&mut quads);
                quads.extend(into_screen(frame.quads));
                game.quads = quads;
                game.hud_prims = frame.hud.prims;
                game.hud_layer = frame.hud.layer;
                if frame.capture {
                    game.xfade_captures += 1;
                }
                game.xfade_alpha = frame.xfade_alpha;
                game.rumble = frame.rumble;
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(reason) = frame.exit {
                    leave_battle(game, &mut commands, &mut audio, reason, winner);
                }
            }
        }
        // FontIcon_Tick.
        game.font.tick_icons(vsyncs);
    }
}

fn into_screen_at(quads: Vec<loading::Quad>, texture_base: usize) -> Vec<ScreenQuad> {
    quads
        .into_iter()
        .map(|mut quad| {
            quad.texture = quad.texture.map(|index| index + texture_base);
            ScreenQuad::from(quad)
        })
        .collect()
}

/// `Battle_Term`, then `Progress_Main` picks the next screen from the result's reason: 8 the main menu, 0x10 the
/// character select (the team select of team battles is not ported), 0x80 the Vs menu; the replay screen (0x1000)
/// is not ported.
fn leave_battle(
    game: &mut Game,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
    reason: u32,
    winner: u32,
) {
    stop_all(game, commands);
    if let Scene::Battle(battle) = &game.scene {
        game.vu0_random = battle.vu0;
        // gBattleReplay: what the replay list saves.
        if let Some((_, replay)) = &mut game.last_battle {
            *replay = battle.replay().clone();
        }
    }
    // BtlLoad_Term: the battle's sample banks.
    for mask in [4, 8, 0x10, 0x20] {
        game.se.unload_bank(mask);
    }
    game.model_batches.clear();
    game.stage_batches.clear();
    game.effects.clear();
    game.under_effects.clear();
    game.shadow_effects.clear();
    game.post_ops.clear();
    game.pre_ops.clear();
    game.depth_tint = None;
    game.hud_prims.clear();
    game.hud_layer = None;
    game.xfade_alpha = 0;
    game.hud_clamp_from = usize::MAX;
    if std::mem::take(&mut game.story_battle) {
        // Progress_Main after a story battle: an abort's reason 8 goes to the main menu, 0x800 to the saga select;
        // else Hist_Main mode 8, the result (won with winner bit 1, and the dragon ball found).
        if winner & 8 != 0 && reason & 8 != 0 {
            start_main_menu(game, commands, audio);
        } else if winner & 8 != 0 && reason & 0x800 != 0 {
            start_hist_select(game, commands);
        } else {
            let ball = game.story_ball;
            start_hist_result(game, winner & 1 != 0, ball);
        }
    } else if let Some(mode) = game.train_mode.take() {
        // Progress_Main after a battle of the training: an abort's reason 8 goes to the main menu, 0x10 to the
        // character select (mode 45); else the mode the battle was set up in, the menu (44) reopening where the
        // progress flags left it.
        if reason & 8 != 0 {
            start_main_menu(game, commands, audio);
        } else if reason & 0x10 != 0 || mode == 45 {
            start_train_select(game, commands);
        } else {
            start_training(game, commands);
        }
    } else if reason & 0x1000 != 0 {
        // Progress_Main: "Save Replay Data" goes to the replay list in its save mode.
        game.progress.replay_flags |= 1;
        game.dc.cursor = -1;
        game.dc.replay_cursor = 0;
        start_replay_menu(game, commands);
    } else if std::mem::take(&mut game.in_replay_list) && reason & (8 | 0x10 | 0x80 | 0x800) == 0 {
        // A replay from the list goes back to it (the progress mode is still 56).
        game.in_replay_list = true;
        start_replay_menu(game, commands);
    } else if std::mem::take(&mut game.in_demo) {
        // Progress_Main with the title's mode: the result's reasons count only for an abort (winner bit 8), which
        // the attract battle's START gives with reason 0.
        start_title(game, commands, audio);
    } else if reason & 8 != 0 {
        start_main_menu(game, commands, audio);
    } else if reason & 0x10 != 0 && game.duel.battle_type == 0 {
        start_char_select(game, commands);
    } else if reason & 0x10 != 0 {
        // Progress_Main: a team or DP battle's "Character Select" goes to mode 40.
        start_team_select(game, commands);
    } else if reason & 0x80 != 0 {
        start_duel_menu(game, commands);
    } else {
        info!("the screen after the battle (reason {reason:#X}) is not ported yet");
        game.scene = Scene::Black;
    }
}

/// `Battle_Main` from `Battle_Init`: `Battle_Load` shows the loading screen (it takes over the shared generators)
/// while the files are read.
/// The battle `--battle` / `--mode` start: 1P vs COM, pad 0 against the computer (2P in split screen with
/// `--split`). Survival (mode 3) plays the
/// course's opponents (`SurvSel_SetupBattle` from menu archive 3, section 2); free training (mode 6) takes the
/// character select's rule for it (announcer 1, no time limit, no stage change, changes allowed). The other modes keep the versus
/// rule under their own mode, whose rules `BattleSetup_FinishEx` applies.
fn quick_battle_setup(game: &mut Game) -> Result<BattleSetup, Box<dyn std::error::Error>> {
    let (mode, course) = game.battle_mode;
    if mode == 3 {
        let mut image = Iso9660::open(&game.iso_path)?;
        let pack = read_file(&mut image, duel_menu::ARCHIVE_FILE_ID + 3)?;
        let res = bpe::decode(bt3_data::pack_section(&pack, 2)?)?;
        let data = bt3_data::survival::SurvivalData::parse(&res)?;
        return BattleSetup::survival(
            &data,
            course,
            (0, 0, [0; 8]),
            &mut game.random,
            &mut game.libc_random,
        )
        .ok_or_else(|| "the survival course names a missing opponent".into());
    }
    let mut setup = BattleSetup {
        mode,
        stage: game.battle_stage,
        ..BattleSetup::default()
    };
    setup.sides[1].chara = 1;
    setup.sides[1].control = 2;
    setup.sides[1].pad = 1;
    // --split: the character select's 2P rule (pad 2 plays side 1, split screen).
    if game.battle_split {
        setup.screen_mode = 1;
        setup.sides[1].control = 0;
        for side in &mut setup.sides {
            side.change_allowed = 1;
        }
    }
    if mode == 6 {
        setup.announcer = 1;
        setup.sides[1].cpu_level = -1;
        for side in &mut setup.sides {
            side.change_allowed = 1;
        }
    }
    setup.finish();
    Ok(setup)
}

fn start_battle(game: &mut Game, setup: BattleSetup) {
    info!("battle setup {setup:?}");
    // BattleSetup_FinishEx copies the finished setup into the replay block.
    let mut finished = setup.clone();
    finished.finish();
    game.last_battle = Some((finished, btl_replay::Replay::default()));
    let game_random = std::mem::replace(&mut game.random, GameRandom::from_key(&[0]));
    let libc_random = std::mem::replace(&mut game.libc_random, LibcRandom::new(0));
    let screen = LoadingScreen::new(game_random, libc_random, game.last_loading_type, None);
    game.last_loading_type = Some(screen.screen_type());
    let texture_base = game.textures.len();
    let textures = Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| load_screen_textures(&mut image, screen.screen_type()));
    match textures {
        Ok(textures) => {
            for texture in textures {
                push_texture(game, texture, Sampling::Nearest);
            }
        }
        Err(error) => error!("cannot load the loading screen: {error}"),
    }
    let iso_path = game.iso_path.clone();
    let job_setup = setup.clone();
    let job = std::thread::spawn(move || {
        let setup = job_setup;
        let mut image = Iso9660::open(&iso_path).map_err(|e| e.to_string())?;
        // The executable SYSTEM.CNF boots holds the battle's constant tables.
        let cnf = image.read_file("SYSTEM.CNF").map_err(|e| e.to_string())?;
        let exe = bt3_data::elf::boot_path(&cnf).ok_or("SYSTEM.CNF names no executable")?;
        let elf = image
            .read_file(&exe)
            .and_then(bt3_data::elf::Elf::parse)
            .map_err(|e| e.to_string())?;
        let mut read = |id: u32| read_file(&mut image, id).map_err(|e| e.to_string());
        battle::load(&mut read, &elf, &setup)
    });
    game.model_batches.clear();
    game.stage_batches.clear();
    game.effects.clear();
    game.under_effects.clear();
    game.shadow_effects.clear();
    game.post_ops.clear();
    game.pre_ops.clear();
    game.depth_tint = None;
    game.hud_prims.clear();
    game.hud_layer = None;
    game.scene = Scene::BattleLoad(Box::new(BattleLoad {
        screen,
        texture_base,
        job: Some(job),
        setup,
    }));
}

/// After `Load_RunBlocking`: the generators go back, the objects are created and `Battle_ResetWork` starts the
/// music.
fn finish_battle_load(game: &mut Game, commands: &mut Commands) {
    let Scene::BattleLoad(load) = std::mem::replace(&mut game.scene, Scene::Black) else {
        return;
    };
    let BattleLoad {
        screen, job, setup, ..
    } = *load;
    (game.random, game.libc_random) = screen.into_random();
    let assets = match job.map(JoinHandle::join) {
        Some(Ok(Ok(assets))) => assets,
        Some(Ok(Err(error))) => {
            error!("cannot load the battle: {error}");
            return;
        }
        _ => {
            error!("the battle loader stopped");
            return;
        }
    };
    game.model_textures = (assets.base_textures.clone(), assets.shade_textures.clone());
    game.model_rev += 1;
    game.stage_textures = assets.stage_textures.clone();
    game.stage_rev = assets.stage_rev;
    game.effect_textures = assets.effect_textures.clone();
    game.effect_point = assets.shadow_pages.to_vec();
    game.hud_textures = assets.hud_textures.clone();
    game.hud_clamp_from = assets.menu_sheet.base;
    let mut battle = battle::Battle::new(assets);
    if let Some(replay) = game.pending_replay.take() {
        battle.play_replay(replay);
    }
    if let Some(script) = game.pending_script.take() {
        battle.script = Some(Box::new(script));
        battle.story_difficulty = game.hist.level;
        // BattleSetup_FinishEx in mode 1: rule.dragonBall, 1 + the first dragon ball the save lacks (0: all seven).
        let flags = game.save.unlock_flags();
        let rule = (0..7).find(|i| flags & 1 << i == 0).map_or(0, |i| i + 1);
        battle
            .assets
            .hidden_item
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .rule = rule;
    }
    battle.vu0 = game.vu0_random;
    battle.chara_bits = game.save.chara_bits;
    battle.assets.chara_bits = game.save.chara_bits;
    // BtlLoad_StepChara reads the character changes from the disc.
    let iso_path = game.iso_path.clone();
    let mut image = None;
    battle.loader = Some(Box::new(move |id: u32| {
        if image.is_none() {
            image = Some(Iso9660::open(&iso_path).map_err(|e| e.to_string())?);
        }
        let image = image.as_mut().unwrap();
        read_file(image, id).map_err(|e| e.to_string())
    }));
    game.scene = Scene::Battle(Box::new(battle));
    play_bgm(game, commands, battle::BGM_FIRST + setup.bgm.max(0) as u32);
}

fn into_screen(quads: Vec<loading::Quad>) -> Vec<ScreenQuad> {
    quads.into_iter().map(ScreenQuad::from).collect()
}

/// Stops a player. Its entity may be gone already: one-shot players (`PlaybackSettings::DESPAWN`) despawn themselves
/// when their sound ends.
fn stop_player(player: &mut Option<Player>, commands: &mut Commands) {
    if let Some(player) = player.take() {
        commands.entity(player.entity).try_despawn();
    }
}

/// Waits for a decoder thread and keeps its WAV as an audio asset.
fn finish_decoding(
    decoding: &mut Option<Decoding>,
    source: &mut Option<Handle<AudioSource>>,
    audio: &mut Assets<AudioSource>,
    what: &str,
) {
    if let Some(handle) = decoding.take() {
        match handle.join() {
            Ok(Ok(wav)) => *source = Some(audio.add(AudioSource { bytes: wav.into() })),
            Ok(Err(error)) => error!("cannot decode {what}: {error}"),
            Err(_) => error!("{what} decoder thread panicked"),
        }
    }
}

/// A one-shot ADX player started with its file once the decoder finished: player `channel` (`ADX_CH_*`) at
/// volume `volume` (0..0x80).
fn start_pending(
    pending: &mut Option<(Decoding, i32)>,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
    channel: usize,
    what: &str,
) -> Option<Player> {
    if !pending.as_ref().is_some_and(|(d, _)| d.is_finished()) {
        return None;
    }
    let (decoding, volume) = pending.take()?;
    let mut source = None;
    finish_decoding(&mut Some(decoding), &mut source, audio, what);
    let level = sound::out_volume(channel, volume);
    let settings = PlaybackSettings::DESPAWN.with_volume(sound::volume(level));
    let entity = commands.spawn((AudioPlayer(source?), settings)).id();
    Some(Player { entity, level })
}

/// The "MAP" player's file once decoded, looping at the volume last set.
fn start_pending_map(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    if !game
        .pending_map
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        return;
    }
    let mut source = None;
    finish_decoding(
        &mut game.pending_map,
        &mut source,
        audio,
        "a stage ambience",
    );
    if let Some(source) = source {
        let level = sound::out_volume(sound::CH_MAP, game.map_volume);
        let settings = PlaybackSettings::LOOP.with_volume(sound::volume(level));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.map_bgm = Some(Player { entity, level });
    }
}

/// The streamed-SE players' files once decoded.
fn start_pending_stream_se(
    game: &mut Game,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
) {
    for i in 0..2 {
        let channel = sound::CH_SE0 + i;
        if let Some(p) = start_pending(
            &mut game.pending_stream_se[i],
            commands,
            audio,
            channel,
            "a streamed sound",
        ) {
            game.stream_se[i] = Some(p);
            if i == 0 {
                game.stream_fresh = true;
                game.stream_ended = false;
            }
        }
    }
}

/// The voice players' files once decoded.
fn start_pending_voice(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    for i in 0..2 {
        let channel = sound::CH_VOICE0 + i;
        if let Some(p) = start_pending(
            &mut game.pending_voice[i],
            commands,
            audio,
            channel,
            "a voice",
        ) {
            game.voice[i] = Some(p);
            if i == 0 {
                game.voice_fresh = true;
            }
        }
    }
}

/// `Movie_PlayOpening` (partition 2 is already readable from the ISO).
fn start_opening_movie(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    let Some(texture) = game.movie_texture else {
        game.scene = Scene::Black;
        return;
    };
    let video = match VideoStream::open(&game.iso_path, movie::PSS_PATH) {
        Ok(video) => video,
        Err(error) => {
            error!("cannot decode {} with ffmpeg: {error}", movie::PSS_PATH);
            start_title(game, commands, audio);
            return;
        }
    };
    finish_decoding(
        &mut game.soundtrack,
        &mut game.soundtrack_source,
        audio,
        movie::ADX_PATH,
    );
    if let Some(source) = game.soundtrack_source.clone() {
        let level = sound::out_volume(sound::CH_BGM, 0x40);
        let settings =
            PlaybackSettings::DESPAWN.with_volume(Volume::Decibels(movie::soundtrack_volume_db()));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.bgm = Some(Player { entity, level });
    }
    game.scene = Scene::Movie(MoviePlayer::new(texture), video, false);
}

/// `Hist_Main` mode 3: `Movie_PlayEnding`, from the saga select (`prev_mode` 6) or a saga's outro (9). Its
/// soundtrack is decoded before the movie starts.
fn start_ending_movie(
    game: &mut Game,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
    prev_mode: i32,
) {
    game.ending_from = prev_mode;
    let Some(texture) = game.movie_texture else {
        finish_ending_movie(game, commands);
        return;
    };
    let video = match VideoStream::open(&game.iso_path, movie::ENDING_PSS_PATH) {
        Ok(video) => video,
        Err(error) => {
            error!(
                "cannot decode {} with ffmpeg: {error}",
                movie::ENDING_PSS_PATH
            );
            finish_ending_movie(game, commands);
            return;
        }
    };
    let path = game.iso_path.clone();
    let mut decoding = Some(std::thread::spawn(move || {
        let bytes = Iso9660::open(&path)
            .and_then(|mut iso| iso.read_file(movie::ENDING_ADX_PATH))
            .map_err(|error| error.to_string())?;
        adx_to_wav(&bytes)
    }));
    let mut source = None;
    finish_decoding(&mut decoding, &mut source, audio, movie::ENDING_ADX_PATH);
    if let Some(source) = source {
        let level = sound::out_volume(sound::CH_BGM, 0x40);
        let settings =
            PlaybackSettings::DESPAWN.with_volume(Volume::Decibels(movie::soundtrack_volume_db()));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.bgm = Some(Player { entity, level });
    }
    game.scene = Scene::Movie(MoviePlayer::new(texture), video, true);
}

/// After `Movie_PlayEnding`: saga 8's slot marked seen (flag 1), music 7 unlocked; back to the saga select, or
/// after an outro (slot flag 2) on to the save screen.
fn finish_ending_movie(game: &mut Game, commands: &mut Commands) {
    stop_all(game, commands);
    let flags = game.save.slot_flags(8) | 1;
    game.save.set_slot_flags(8, flags);
    game.save.bgm_bits |= 0x80;
    if game.ending_from == 9 {
        let flags = game.save.slot_flags(8) | 2;
        game.save.set_slot_flags(8, flags);
        start_hist_save(game, 3);
    } else {
        start_hist_select(game, commands);
    }
}

/// Progress_Main mode 1: `Bgm_Play(MENU_BGM_TITLE)`, then `Title_Run(1)`.
fn start_title(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    load_menu_bank(game);
    if game.title.is_none() {
        match load_title_assets(&game.iso_path) {
            Ok(assets) => {
                let fod = assets.fod.clone();
                let dialog_fod = assets.dialog.fod.clone();
                let dialog_titles = assets.dialog.titles.clone();
                let layout = register_title_textures(game, assets);
                game.title = Some(TitleData {
                    fod,
                    dialog_fod,
                    dialog_titles,
                    layout,
                    card_res: game.card_res.clone(),
                });
            }
            Err(error) => {
                error!("cannot load the title screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.title.as_ref().expect("title assets are loaded");
    // gProgress->flags & MPROG_SAVE_LOADED: a save was loaded at boot.
    let loaded = game.save_loaded || game.progress.flags & mcflow::PROGRESS_SAVE_LOADED != 0;
    let screen = match TitleScreen::new(data, loaded, &mut game.random) {
        Ok(screen) => screen,
        Err(error) => {
            error!("cannot start the title movie: {error}");
            game.scene = Scene::Black;
            return;
        }
    };
    play_title_music(game, commands, audio);
    game.scene = Scene::Title(Box::new(screen));
}

/// Progress_Main: Snd_LoadBankFile(2, 0x14C) before the mode loop (a loaded slot is left as it is).
fn load_menu_bank(game: &mut Game) {
    match Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| read_file(&mut image, MENU_SE_BANK_FILE_ID))
        .and_then(|file| Ok(se::LoadedBank::parse(&file)?))
    {
        Ok(bank) => game.se.load_bank(2, bank),
        Err(error) => error!("cannot load the menu sound bank: {error}"),
    }
}

/// `Bgm_Play(MENU_BGM_TITLE)`: the file loops over all of its samples.
fn play_title_music(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    finish_decoding(
        &mut game.music,
        &mut game.music_source,
        audio,
        "the title music",
    );
    if let Some(source) = game.music_source.clone() {
        let level = sound::out_volume(sound::CH_BGM, 0x40);
        let settings = PlaybackSettings::LOOP.with_volume(sound::volume(level));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.bgm = Some(Player { entity, level });
    }
}

/// Progress_Main mode 4: `Bgm_Play(MENU_BGM_TITLE)`, then `MainMenu_Run(2)`.
fn start_main_menu(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    if game.main_menu.is_none() {
        match load_main_menu_assets(&game.iso_path) {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = MainMenuLayout::new(background, |key| firsts[&key]);
                game.main_menu = Some(data);
            }
            Err(error) => {
                error!("cannot load the main menu: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .main_menu
        .as_ref()
        .expect("main menu assets are loaded");
    match MainMenuScreen::new(data, game.main_menu_item, &mut game.random) {
        Ok(screen) => {
            play_title_music(game, commands, audio);
            game.scene = Scene::MainMenu(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the main menu: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `MainMenu_Init(2)`: section 2 of gMenuArc1, BPE-packed.
fn load_main_menu_assets(iso_path: &Path) -> Result<MainMenuAssets, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(iso_path)?;
    let archive = read_file(&mut image, title::ARCHIVE_FILE_ID)?;
    let res = bpe::decode(pack_section(&archive, main_menu::ARCHIVE_SECTION)?)?;
    MainMenuAssets::parse(&res)
}

/// Progress_Main mode 38: `Duel_Main` plays `DUEL_BGM`, then `DuelMenu_Run(1)`.
fn start_duel_menu(game: &mut Game, commands: &mut Commands) {
    if game.duel_menu.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, duel_menu::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    duel_menu::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| DuelMenuAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = DuelMenuLayout::new(background, dialog, |key| firsts[&key]);
                game.duel_menu = Some(data);
            }
            Err(error) => {
                error!("cannot load the duel menu: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .duel_menu
        .as_ref()
        .expect("duel menu assets are loaded");
    match DuelMenuScreen::new(data, game.duel, game.save.rules, &mut game.random) {
        Ok(screen) => {
            play_bgm(game, commands, duel_menu::BGM_FILE_ID);
            game.scene = Scene::DuelMenu(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the duel menu: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `CharRefMode_Main` mode 60: `Bgm_Play(BGM_CHAR_REF)`, then `CharRef_Run(1)` from `gMenuArc9`.
fn start_char_ref(game: &mut Game, commands: &mut Commands) {
    if game.char_ref.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, char_ref::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    char_ref::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| char_ref::CharRefAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = char_ref::CharRefLayout::new(background, |key| firsts[&key]);
                game.char_ref = Some(data);
            }
            Err(error) => {
                error!("cannot load the character reference: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .char_ref
        .clone()
        .expect("character reference assets are loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    match char_ref::CharRefScreen::new(&data, &game.save, &game.chr_table, &mut loader) {
        Ok(screen) => {
            // ChrView_Init: the backdrop read at once; the faces' tables come from the executable.
            let viewer = (|| -> Result<chr_view::ChrView, String> {
                let mut image = Iso9660::open(&game.iso_path).map_err(|e| e.to_string())?;
                let cnf = image.read_file("SYSTEM.CNF").map_err(|e| e.to_string())?;
                let exe = bt3_data::elf::boot_path(&cnf).ok_or("SYSTEM.CNF names no executable")?;
                let elf = image
                    .read_file(&exe)
                    .and_then(bt3_data::elf::Elf::parse)
                    .map_err(|e| e.to_string())?;
                let face = btl_face::FaceTables::read(&elf).map_err(|e| e.to_string())?;
                let mut read = |id: u32| read_file(&mut image, id).map_err(|e| e.to_string());
                chr_view::ChrView::new(&mut read, face, &game.chr_table)
            })();
            match viewer {
                Ok(viewer) => game.chr_view = Some(viewer),
                Err(error) => error!("cannot start the model viewer: {error}"),
            }
            game.chr_view_rev = None;
            play_bgm(game, commands, char_ref::BGM_FILE_ID);
            game.scene = Scene::CharRef(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the character reference: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Tour_Main` mode 33: `Bgm_Play(0x10B19)`, then `TourMenu_Run(1)` from `gMenuArc4`.
fn start_tour_menu(game: &mut Game, commands: &mut Commands) {
    if game.tour_menu.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, tour_menu::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    tour_menu::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| tour_menu::TourMenuAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = tour_menu::TourMenuLayout::new(background, |key| firsts[&key]);
                game.tour_menu = Some(data);
            }
            Err(error) => {
                error!("cannot load the tournament menu: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .tour_menu
        .clone()
        .expect("the tournament menu is loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    let mut sounds = Vec::new();
    match tour_menu::TourMenuScreen::new(
        &data,
        &mut game.save,
        &mut loader,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            play_bgm(game, commands, tour_menu::BGM_FILE_ID);
            game.scene = Scene::TourMenu(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the tournament menu: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `EvoMode_Main` mode 49: `EvoZ_Run(3)` from `gMenuArc7`.
fn start_evo_z(game: &mut Game) {
    let file4 = match Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| read_file(&mut image, 4))
    {
        Ok(file) => file,
        Err(error) => {
            error!("cannot read the character table: {error}");
            game.scene = Scene::Black;
            return;
        }
    };
    if game.chr_table.costumes.is_empty() {
        game.chr_table = bt3_data::chara_data::ChrTable::parse(&file4, 0xA1);
    }
    let params = password::section_bytes(&file4, 2)
        .map(password::item_params)
        .unwrap_or_default();
    if game.evo_z.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, evo_top::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    evo_z::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| evo_z::EvoZAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = evo_z::EvoZLayout::new(dialog, |key| firsts[&key]);
                game.evo_z = Some(data);
            }
            Err(error) => {
                error!("cannot load the customising screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .evo_z
        .clone()
        .expect("the customising screen is loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    match evo_z::EvoZScreen::new(
        &data,
        &game.chr_table,
        params,
        &mut game.save,
        &mut game.char_progress.side[0],
        &mut loader,
    ) {
        Ok(screen) => game.scene = Scene::EvoZ(Box::new(screen)),
        Err(error) => {
            error!("cannot start the customising screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `EvoMode_Main` mode 50: `Shop_Run(2)` from `gMenuArc7`.
fn start_shop(game: &mut Game) {
    if game.chr_table.costumes.is_empty() {
        match Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, 4))
        {
            Ok(file) => game.chr_table = bt3_data::chara_data::ChrTable::parse(&file, 0xA1),
            Err(error) => error!("cannot read the character table: {error}"),
        }
    }
    if game.shop.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, evo_top::ARCHIVE_FILE_ID))
            .and_then(|archive| Ok(bpe::decode(pack_section(&archive, shop::ARCHIVE_SECTION)?)?))
            .and_then(|res| shop::ShopAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = shop::ShopLayout::new(background, |key| firsts[&key]);
                game.shop = Some(data);
            }
            Err(error) => {
                error!("cannot load the shop: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.shop.clone().expect("the shop is loaded");
    match shop::ShopScreen::new(&data, &game.chr_table, &mut game.save, &mut game.random) {
        Ok(screen) => game.scene = Scene::Shop(Box::new(screen)),
        Err(error) => {
            error!("cannot start the shop: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `EvoMode_Main` mode 48: the music once, then `EvoTop_Run(1)` from `gMenuArc7`.
fn start_evo_top(game: &mut Game, commands: &mut Commands) {
    if game.evo_top.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, evo_top::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    evo_top::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| evo_top::EvoTopAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = evo_top::EvoTopLayout::new(background, dialog, |key| firsts[&key]);
                game.evo_top = Some(data);
            }
            Err(error) => {
                error!("cannot load Evolution Z's menu: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.evo_top.clone().expect("Evolution Z's menu is loaded");
    let mut sounds = Vec::new();
    match evo_top::EvoTopScreen::new(
        &data,
        game.evo_cursor,
        game.card_res.as_deref(),
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            if !std::mem::replace(&mut game.evo_bgm, true) {
                play_bgm(game, commands, evo_top::BGM_FILE_ID);
            }
            game.scene = Scene::EvoTop(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start Evolution Z's menu: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Shen_Main` mode 70: `Bgm_Play(0x10B18)`, `Pad_SetRepeat(20, 1)` and `Shen_Run(2)` from `gMenuArc12`.
fn start_shen(game: &mut Game, commands: &mut Commands) {
    if game.chr_table.costumes.is_empty() {
        match Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, 4))
        {
            Ok(file) => game.chr_table = bt3_data::chara_data::ChrTable::parse(&file, 0xA1),
            Err(error) => error!("cannot read the character table: {error}"),
        }
    }
    if game.shen.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, shen::ARCHIVE_FILE_ID))
            .and_then(|archive| Ok(bpe::decode(pack_section(&archive, shen::ARCHIVE_SECTION)?)?))
            .and_then(|res| shen::ShenAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let firsts = register_files(game, assets.files);
                let cfm_firsts = register_files(game, assets.cfm_files);
                data.layout = shen::ShenLayout::new(|key| firsts[&key], |key| cfm_firsts[&key]);
                game.shen = Some(data);
            }
            Err(error) => {
                error!("cannot load the wish screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.shen.clone().expect("wish screen assets are loaded");
    let iso_path = game.iso_path.clone();
    let mut read = |id: u32| {
        Iso9660::open(&iso_path)
            .map_err(|e| e.to_string())
            .and_then(|mut image| read_file(&mut image, id).map_err(|e| e.to_string()))
    };
    match shen::ShenScreen::new(&data, &mut game.save, &mut game.random, &mut read) {
        Ok(screen) => {
            play_bgm(game, commands, shen::BGM_FILE_ID);
            game.shen_textures = false;
            game.scene = Scene::Shen(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the wish screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Shen_Main` mode 71: `ShenSave_Run(2)`, the dialog from section 10 of `gMenuArc12`'s section 2.
fn start_shen_save(game: &mut Game) {
    let loaded = Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| read_file(&mut image, shen::ARCHIVE_FILE_ID))
        .and_then(|file| Ok(bpe::decode(pack_section(&file, shen::ARCHIVE_SECTION)?)?))
        .and_then(|res| dialog::DialogAssets::parse(pack_section(&res, shen::DIALOG_SECTION)?));
    let dialog = match loaded {
        Ok(assets) => {
            let indices = register_pairs(game, assets.textures);
            dialog::Dialog::new(&assets.fod, assets.titles, indices, None, 0)
        }
        Err(error) => Err(error),
    };
    match dialog {
        Ok(dialog) => {
            let screen = shen::ShenSaveScreen::new(dialog, game.card_res.as_deref());
            game.scene = Scene::ShenSave(Box::new(screen));
        }
        Err(error) => {
            error!("cannot load the wish screen's save: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `TrainMode_Main` mode 44: `Bgm_Play(BGM_TRAIN)`, then `Train_Run(1)` from `gMenuArc6`.
fn start_training(game: &mut Game, commands: &mut Commands) {
    if game.train.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, training::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    training::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| training::TrainAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = training::TrainLayout::new(background, |key| firsts[&key]);
                game.train = Some(data);
            }
            Err(error) => {
                error!("cannot load the training menu: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.train.clone().expect("training assets are loaded");
    match training::TrainScreen::new(&data, game.progress.train, &mut game.random) {
        Ok(screen) => {
            play_bgm(game, commands, training::BGM_FILE_ID);
            game.scene = Scene::Training(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the training menu: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `TrainMode_Main` mode 45: `CharSel_Run(2)` from `gMenuArc6`, the character select of free training (it starts
/// its own music).
fn start_train_select(game: &mut Game, commands: &mut Commands) {
    if game.chr_table.costumes.is_empty() {
        match Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, 4))
        {
            Ok(file) => game.chr_table = bt3_data::chara_data::ChrTable::parse(&file, 0xA1),
            Err(error) => error!("cannot read the character table: {error}"),
        }
    }
    if game.train_char_select.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, training::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    training::CHAR_SELECT_SECTION,
                )?)?)
            })
            .and_then(|res| CharSelAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let firsts = register_files(game, assets.files);
                data.layout = CharSelLayout::new(|key| firsts[&key]);
                game.train_char_select = Some(data);
            }
            Err(error) => {
                error!("cannot load the training's character select: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .train_char_select
        .as_ref()
        .expect("the training's character select is loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    let mut sounds = Vec::new();
    // CharSel_Init: CSPROG->players = 0 in mode 0x2D.
    match CharSelScreen::new(
        data,
        0,
        game.char_progress,
        &game.save,
        game.chr_table.clone(),
        &mut loader,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            game.train_mode = Some(45);
            game.scene = Scene::CharSelect(Box::new(screen.for_training()));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the training's character select: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `OptMode_Main` mode 62: `Bgm_Play(BGM_OPTION)`, then `Option_Run(1)` from `gMenuArc10`.
fn start_options(game: &mut Game, commands: &mut Commands) {
    if game.options.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, options::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    options::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| options::OptionAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = options::OptionLayout::new(background, dialog, |key| firsts[&key]);
                game.options = Some(data);
            }
            Err(error) => {
                error!("cannot load the options screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game.options.as_ref().expect("options assets are loaded");
    match options::OptionScreen::new(data, &game.save, game.card_res.as_deref()) {
        Ok(screen) => {
            play_bgm(game, commands, options::BGM_FILE_ID);
            game.scene = Scene::Options(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the options screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `gMenuArc8` (file `gProgress->baseFile + 0xB`), unpacked section `section`.
fn load_dc_section(iso_path: &Path, section: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(iso_path)?;
    let archive = read_file(&mut image, dc_menu::ARCHIVE_FILE_ID)?;
    Ok(bpe::decode(pack_section(&archive, section)?)?)
}

/// `Dc_Main` mode 53: `Bgm_Play(DC_BGM)`, then `DcMenu_Run(1)`.
fn start_dc_menu(game: &mut Game, commands: &mut Commands) {
    if game.dc_menu.is_none() {
        match load_dc_section(&game.iso_path, dc_menu::ARCHIVE_SECTION)
            .and_then(|res| dc_menu::DcMenuAssets::parse(&res))
        {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = dc_menu::DcMenuLayout::new(background, dialog, |key| firsts[&key]);
                game.dc_menu = Some(data);
            }
            Err(error) => {
                error!("cannot load the Data Center: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .dc_menu
        .as_ref()
        .expect("Data Center assets are loaded");
    match dc_menu::DcMenuScreen::new(data, game.card_res.as_deref()) {
        Ok(screen) => {
            play_bgm(game, commands, dc_menu::BGM_FILE_ID);
            game.scene = Scene::DcMenu(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the Data Center: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Dc_Main` mode 54: `Bgm_Play(DC_BGM)`, then `DcPass_Run(3)`.
fn start_dc_pass(game: &mut Game, commands: &mut Commands) {
    if game.dc_pass.is_none() {
        match load_dc_section(&game.iso_path, dc_password::ARCHIVE_SECTION)
            .and_then(|res| dc_password::DcPassAssets::parse(&res))
        {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout =
                    dc_password::DcPassLayout::new(background, dialog, |key| firsts[&key]);
                game.dc_pass = Some(data);
            }
            Err(error) => {
                error!("cannot load the password screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .dc_pass
        .as_ref()
        .expect("password screen assets are loaded");
    match dc_password::DcPassScreen::new(data, &game.chara_table, &mut game.random) {
        Ok(screen) => {
            play_bgm(game, commands, dc_menu::BGM_FILE_ID);
            game.scene = Scene::DcPass(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the password screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Dc_Main` mode 55: `Bgm_Play(DC_BGM)`, then `DcList_Run(2)`.
fn start_dc_list(game: &mut Game, commands: &mut Commands) {
    if game.chr_table.costumes.is_empty() {
        game.chr_table = bt3_data::chara_data::ChrTable::parse(&game.chara_table, 0xA1);
    }
    if game.dc_list.is_none() {
        match load_dc_section(&game.iso_path, dc_list::ARCHIVE_SECTION)
            .and_then(|res| dc_list::DcListAssets::parse(&res))
        {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout = dc_list::DcListLayout::new(background, dialog, |key| firsts[&key]);
                game.dc_list = Some(data);
            }
            Err(error) => {
                error!("cannot load the custom character list: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .dc_list
        .as_ref()
        .expect("character list assets are loaded");
    match dc_list::DcListScreen::new(data, &game.save, &game.chara_table, &mut game.random) {
        Ok(screen) => {
            play_bgm(game, commands, dc_menu::BGM_FILE_ID);
            game.scene = Scene::DcList(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the custom character list: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Dc_Main` mode 56: `Bgm_Play(DC_BGM)`, then `ReplayMenu_Run(4)` (saving the last battle's replay when the
/// progress block's replay flag asks for it).
fn start_replay_menu(game: &mut Game, commands: &mut Commands) {
    if game.replay_menu.is_none() {
        match load_dc_section(&game.iso_path, replay_menu::ARCHIVE_SECTION)
            .and_then(|res| replay_menu::ReplayMenuAssets::parse(&res))
        {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                let dialog = register_pairs(game, assets.dialog.textures);
                data.layout =
                    replay_menu::ReplayMenuLayout::new(background, dialog, |key| firsts[&key]);
                game.replay_menu = Some(data);
            }
            Err(error) => {
                error!("cannot load the replay list: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .replay_menu
        .as_ref()
        .expect("replay list assets are loaded");
    let rep = game
        .last_battle
        .as_ref()
        .map(|(s, r)| (s, r, game.chara_table.as_slice()));
    let mut mc = mcflow::McEnv {
        card: &mut game.card,
        save: &mut game.save,
        progress: &mut game.progress,
        items: &game.chr_table.items,
    };
    match replay_menu::ReplayMenuScreen::new(
        data,
        game.card_res.as_deref(),
        rep,
        game.dc.replay_cursor,
        game.duel.battle_type,
        &mut mc,
    ) {
        Ok(screen) => {
            play_bgm(game, commands, dc_menu::BGM_FILE_ID);
            game.scene = Scene::ReplayMenu(Box::new(screen));
        }
        Err(error) => {
            error!("cannot start the replay list: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Bgm_Play(id)` for a file other than the title music: decoded in the background, looped over all of its samples.
fn play_bgm(game: &mut Game, commands: &mut Commands, file_id: u32) {
    if let Some(source) = game.bgm_sources.get(&file_id).cloned() {
        spawn_bgm(game, commands, source);
    } else {
        game.pending_bgm = Some((file_id, decode_adx_file(game.iso_path.clone(), file_id)));
    }
}

fn spawn_bgm(game: &mut Game, commands: &mut Commands, source: Handle<AudioSource>) {
    let level = sound::out_volume(sound::CH_BGM, 0x40);
    let settings = PlaybackSettings::LOOP.with_volume(sound::volume(level));
    let entity = commands.spawn((AudioPlayer(source), settings)).id();
    game.bgm = Some(Player { entity, level });
}

fn start_pending_bgm(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    if !game
        .pending_bgm
        .as_ref()
        .is_some_and(|(_, handle)| handle.is_finished())
    {
        return;
    }
    let (file_id, handle) = game.pending_bgm.take().expect("checked above");
    let (mut decoding, mut source) = (Some(handle), None);
    finish_decoding(&mut decoding, &mut source, audio, "menu music");
    if let Some(source) = source {
        game.bgm_sources.insert(file_id, source.clone());
        spawn_bgm(game, commands, source);
    }
}

fn push_texture(game: &mut Game, texture: RgbaTexture, sampling: Sampling) -> usize {
    push_to(&mut game.textures, &mut game.samplers, texture, sampling)
}

fn push_to(
    textures: &mut Vec<RgbaTexture>,
    samplers: &mut Vec<Sampling>,
    texture: RgbaTexture,
    sampling: Sampling,
) -> usize {
    textures.push(texture);
    samplers.push(sampling);
    textures.len() - 1
}

/// The character select's streamed pictures, read from the ISO once and kept.
struct Pictures<'a> {
    iso_path: &'a Path,
    textures: &'a mut Vec<RgbaTexture>,
    samplers: &'a mut Vec<Sampling>,
    cache: &'a mut HashMap<u32, usize>,
}

impl Pictures<'_> {
    /// `File_LoadSync` + `Sprite_Unpack`: the file's texture resource, decoded.
    fn load(&mut self, file_id: u32) -> Option<Vec<RgbaTexture>> {
        let loaded = Iso9660::open(self.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, file_id))
            .and_then(|file| Ok(bpe::decode(&file)?))
            .and_then(|res| decode_textures(&res));
        match loaded {
            Ok(textures) => Some(textures),
            Err(error) => {
                error!("cannot load file 0x{file_id:X}: {error}");
                None
            }
        }
    }
}

impl char_select::PictureLoader for Pictures<'_> {
    fn portrait(&mut self, file_id: u32) -> Option<usize> {
        if let Some(&index) = self.cache.get(&file_id) {
            return Some(index);
        }
        let texture = self.load(file_id)?.into_iter().next()?;
        let index = push_to(
            self.textures,
            self.samplers,
            texture.clone(),
            Sampling::Linear,
        );
        push_to(
            self.textures,
            self.samplers,
            texture,
            Sampling::LinearRepeat,
        );
        self.cache.insert(file_id, index);
        Some(index)
    }

    fn texture_file(&mut self, file_id: u32) -> Option<usize> {
        // Cached apart from the files read as portraits.
        let key = file_id | 0x8000_0000;
        if let Some(&index) = self.cache.get(&key) {
            return Some(index);
        }
        let textures = self.load(file_id)?;
        let first = self.textures.len();
        for texture in textures {
            push_to(
                self.textures,
                self.samplers,
                texture.clone(),
                Sampling::Linear,
            );
            push_to(
                self.textures,
                self.samplers,
                texture,
                Sampling::LinearRepeat,
            );
        }
        self.cache.insert(key, first);
        Some(first)
    }

    fn picture(&mut self, file_id: u32) -> Option<usize> {
        if let Some(&index) = self.cache.get(&file_id) {
            return Some(index);
        }
        let tiles = self.load(file_id)?;
        if tiles.len() != logo::TILES_PER_PICTURE {
            error!("file 0x{file_id:X} has {} tiles", tiles.len());
            return None;
        }
        let first = self.textures.len();
        for tile in tiles {
            push_to(self.textures, self.samplers, tile, Sampling::Nearest);
        }
        self.cache.insert(file_id, first);
        Some(first)
    }
}

/// Progress_Main mode 39: `CharSel_Run(2)` (it starts its own music).
/// Common file 4's character table and the duel archive's section 2, shared by both character selects; false
/// (and a black screen) when they cannot be read.
fn load_char_select(game: &mut Game) -> bool {
    if game.chr_table.costumes.is_empty() {
        // Common file 4: the character entries and the item table.
        match Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, 4))
        {
            Ok(file) => game.chr_table = bt3_data::chara_data::ChrTable::parse(&file, 0xA1),
            Err(error) => error!("cannot read the character table: {error}"),
        }
    }
    if game.char_select.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, duel_menu::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    char_select::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| CharSelAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let firsts = register_files(game, assets.files);
                data.layout = CharSelLayout::new(|key| firsts[&key]);
                game.char_select = Some(data);
            }
            Err(error) => {
                error!("cannot load the character select: {error}");
                game.scene = Scene::Black;
                return false;
            }
        }
    }
    true
}

fn start_char_select(game: &mut Game, commands: &mut Commands) {
    if !load_char_select(game) {
        return;
    }
    let data = game
        .char_select
        .as_ref()
        .expect("character select assets are loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    let mut sounds = Vec::new();
    match CharSelScreen::new(
        data,
        game.duel.players,
        game.char_progress,
        &game.save,
        game.chr_table.clone(),
        &mut loader,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            game.scene = Scene::CharSelect(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the character select: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Hist_Main` mode 6: the menu music, then `HistSel_Run(1)` from `gMenuArc2`.
fn start_hist_select(game: &mut Game, commands: &mut Commands) {
    if game.hist_select.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, hist_select::ARCHIVE_FILE_ID))
            .and_then(|archive| {
                Ok(bpe::decode(pack_section(
                    &archive,
                    hist_select::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| hist_select::HistSelAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = hist_select::HistSelLayout::new(background, |key| firsts[&key]);
                game.hist_select = Some(data);
            }
            Err(error) => {
                error!("cannot load the saga select: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    let data = game
        .hist_select
        .as_ref()
        .expect("saga select assets are loaded");
    let mut sounds = Vec::new();
    match hist_select::HistSelScreen::new(
        data,
        game.hist,
        &game.save,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            play_bgm(game, commands, hist_select::BGM_FILE_ID);
            game.scene = Scene::HistSelect(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the saga select: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `File_LoadSync(baseFile + 0x10 + saga)` + `Sprite_Unpack`: the saga's own pack (with the episode list's section,
/// whose textures its layout shares); false (and a black screen) when they cannot be read.
fn load_saga(game: &mut Game, saga: usize) -> bool {
    if game.mode_menu.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, hist_select::ARCHIVE_FILE_ID))
            .and_then(|file| {
                Ok(bpe::decode(pack_section(
                    &file,
                    mode_menu::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| mode_menu::ModeMenuAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let firsts = register_files(game, assets.files);
                data.msg = mode_menu::ModeMenuAssets::msg_layout(|key| firsts[&key]);
                game.mode_menu = Some((data, firsts));
            }
            Err(error) => {
                error!("cannot load the episode list: {error}");
                game.scene = Scene::Black;
                return false;
            }
        }
    }
    if game.sagas.contains_key(&saga) {
        return true;
    }
    let loaded = Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| read_file(&mut image, mode_menu::SAGA_FILE + saga as u32))
        .and_then(|file| Ok(bpe::decode(&file)?))
        .and_then(|res| mode_menu::SagaAssets::parse(saga, &res));
    match loaded {
        Ok(assets) => {
            let mut data = assets.data;
            let background = register_picture(game, assets.background);
            let own = register_files(game, assets.files);
            let screen = &game.mode_menu.as_ref().expect("episode list is loaded").1;
            data.layout = mode_menu::SagaLayout::new(saga, background, |key| {
                own.get(&key)
                    .or_else(|| screen.get(&key))
                    .copied()
                    .unwrap_or(0)
            });
            game.sagas.insert(saga, data);
            true
        }
        Err(error) => {
            error!("cannot load saga {saga}'s pack: {error}");
            game.scene = Scene::Black;
            false
        }
    }
}

/// `ModeMenu_Run`'s battle (`BattleSetup_SetScript`): the episode's script sets the battle up (`BattleLoad_Step` 4:
/// action 1000, then the text and lip files it asked for), then `Battle_Main`.
fn start_story_battle(game: &mut Game, script: i32) {
    let iso_path = game.iso_path.clone();
    let voice_set = game.save.sound_options().alt_voice;
    let level = game.hist.level;
    let loaded = Iso9660::open(&iso_path)
        .map_err(Box::<dyn std::error::Error>::from)
        .and_then(|mut image| {
            let file = read_file(&mut image, btl_script::SCRIPT_FILE + script.max(0) as u32)?;
            let gsc = bt3_data::gsc::GscScript::parse(&file)?;
            let mut s = btl_script::BtlScript::setup(
                gsc,
                level,
                voice_set,
                &mut game.hist.reward,
                &mut game.libc_random,
            );
            if let Some(id) = s.script.text_file {
                s.script.set_text_file(read_file(&mut image, id)?);
            }
            if let Some(id) = s.script.lip_file {
                s.script.set_lip_file(read_file(&mut image, id)?);
            }
            Ok(s)
        });
    match loaded {
        Ok(s) => {
            game.pending_script = Some(s.script);
            game.story_battle = true;
            start_battle(game, s.setup);
        }
        Err(error) => {
            error!("cannot set up story battle {script}: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Hist_Main` mode 8: `HistResult_Run(3)` after a story battle, won or lost (`ball` the dragon ball it found,
/// 1-based, 0 none).
fn start_hist_result(game: &mut Game, won: bool, ball: i32) {
    let saga = game.hist.sub_menu.clamp(0, 7) as usize;
    if !load_saga(game, saga) {
        return;
    }
    if game.hist_result.is_none() {
        let loaded = Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, hist_select::ARCHIVE_FILE_ID))
            .and_then(|file| {
                Ok(bpe::decode(pack_section(
                    &file,
                    hist_result::ARCHIVE_SECTION,
                )?)?)
            })
            .and_then(|res| hist_result::HistResultAssets::parse(&res));
        match loaded {
            Ok(assets) => {
                let mut data = assets.data;
                let background = register_picture(game, assets.background);
                let firsts = register_files(game, assets.files);
                data.layout = hist_result::HistResultLayout::new(background, |key| firsts[&key]);
                game.hist_result = Some(data);
            }
            Err(error) => {
                error!("cannot load the story result screen: {error}");
                game.scene = Scene::Black;
                return;
            }
        }
    }
    // The saga pack's section 2 entry 1 (the episode list's image 36).
    let picture = game.sagas[&saga].layout.flash.get(36).copied().flatten();
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    let data = game.hist_result.as_ref().expect("result screen is loaded");
    match hist_result::HistResultScreen::new(
        data,
        &game.hist,
        won,
        ball,
        picture,
        &mut game.save,
        &mut loader,
        &mut game.random,
    ) {
        Ok(screen) => game.scene = Scene::HistResult(Box::new(screen)),
        Err(error) => {
            error!("cannot start the story result screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Hist_Main` mode 9: `HistOutro_Run(2)` for the saga `game.hist.sub_menu` (it starts its own music).
fn start_hist_outro(game: &mut Game, commands: &mut Commands) {
    let saga = game.hist.sub_menu.clamp(0, 7) as usize;
    if !load_saga(game, saga) {
        return;
    }
    let (data, _) = game.mode_menu.as_ref().expect("episode list is loaded");
    let mut sounds = Vec::new();
    match hist_outro::HistOutroScreen::new(
        data,
        &game.sagas[&saga],
        &mut game.save,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            game.scene = Scene::HistOutro(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the saga's outro: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Hist_Main` mode 10: `HistSave_Run(4)`, the save flow over a black screen; `prev_mode` is the mode it comes
/// from.
fn start_hist_save(game: &mut Game, prev_mode: i32) {
    let loaded = Iso9660::open(&game.iso_path)
        .map_err(Into::into)
        .and_then(|mut image| read_file(&mut image, hist_select::ARCHIVE_FILE_ID))
        .and_then(|file| {
            Ok(bpe::decode(pack_section(
                &file,
                hist_save::ARCHIVE_SECTION,
            )?)?)
        })
        .and_then(|res| {
            dialog::DialogAssets::parse(pack_section(&res, hist_save::DIALOG_SECTION)?)
        });
    let dialog = match loaded {
        Ok(assets) => {
            let indices = register_pairs(game, assets.textures);
            dialog::Dialog::new(&assets.fod, assets.titles, indices, None, 0)
        }
        Err(error) => Err(error),
    };
    match dialog {
        Ok(dialog) => {
            let screen =
                hist_save::HistSaveScreen::new(dialog, game.card_res.as_deref(), prev_mode);
            game.scene = Scene::HistSave(Box::new(screen));
        }
        Err(error) => {
            error!("cannot load the story save screen: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Hist_Main` mode 7: `ModeMenu_Run(2)` for the saga `game.hist.sub_menu`, from `gMenuArc2` and the saga's pack
/// (it starts its own music); `prev_mode` is the mode it comes from.
fn start_mode_menu(game: &mut Game, commands: &mut Commands, prev_mode: i32) {
    let saga = game.hist.sub_menu.clamp(0, 7) as usize;
    if !load_saga(game, saga) {
        return;
    }
    let (data, _) = game.mode_menu.as_ref().expect("episode list is loaded");
    let mut sounds = Vec::new();
    match mode_menu::ModeMenuScreen::new(
        data,
        &game.sagas[&saga],
        game.hist.sub_menu_item,
        prev_mode,
        &game.save,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            game.scene = Scene::ModeMenu(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the episode list: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// `Duel_Main` mode 40: `TeamSel_Run(2)`.
fn start_team_select(game: &mut Game, commands: &mut Commands) {
    if !load_char_select(game) {
        return;
    }
    let data = game
        .char_select
        .as_ref()
        .expect("character select assets are loaded");
    let mut loader = Pictures {
        iso_path: &game.iso_path,
        textures: &mut game.textures,
        samplers: &mut game.samplers,
        cache: &mut game.pictures,
    };
    let mut sounds = Vec::new();
    match team_select::TeamSelScreen::new(
        data,
        game.duel,
        game.char_progress,
        &game.save,
        game.chr_table.clone(),
        &mut loader,
        &mut game.random,
        &mut sounds,
    ) {
        Ok(screen) => {
            game.scene = Scene::TeamSelect(Box::new(screen));
            play_sounds(game, commands, sounds);
        }
        Err(error) => {
            error!("cannot start the team select: {error}");
            game.scene = Scene::Black;
        }
    }
}

/// A `Sprite_DrawPicture` picture: its first tile's index.
fn register_picture(game: &mut Game, tiles: Vec<RgbaTexture>) -> usize {
    tiles
        .into_iter()
        .map(|texture| push_texture(game, texture, Sampling::Nearest))
        .min()
        .expect("a picture has tiles")
}

/// Flash textures: each one clamped, then wrapping.
fn register_pairs(game: &mut Game, textures: Vec<Option<RgbaTexture>>) -> Vec<Option<usize>> {
    textures
        .into_iter()
        .map(|texture| {
            texture.map(|texture| {
                let index = push_texture(game, texture.clone(), Sampling::Linear);
                push_texture(game, texture, Sampling::LinearRepeat);
                index
            })
        })
        .collect()
}

/// Whole texture files as Flash texture pairs: where entry 0 of each went.
fn register_files(
    game: &mut Game,
    files: Vec<(menu_res::FileKey, Vec<RgbaTexture>)>,
) -> HashMap<menu_res::FileKey, usize> {
    files
        .into_iter()
        .map(|(key, textures)| {
            let first = game.textures.len();
            register_pairs(game, textures.into_iter().map(Some).collect());
            (key, first)
        })
        .collect()
}

fn register_title_textures(game: &mut Game, assets: TitleAssets) -> TextureLayout {
    let mut push = |texture: RgbaTexture, sampling: Sampling| {
        game.textures.push(texture);
        game.samplers.push(sampling);
        game.textures.len() - 1
    };
    let background = assets
        .background
        .into_iter()
        .map(|texture| push(texture, Sampling::Nearest))
        .min();
    let logo = assets
        .logo
        .into_iter()
        .map(|texture| push(texture, Sampling::Nearest))
        .min();
    let flash = assets
        .flash
        .into_iter()
        .map(|texture| {
            texture.map(|texture| {
                let index = push(texture.clone(), Sampling::Linear);
                push(texture, Sampling::LinearRepeat);
                index
            })
        })
        .collect();
    let dialog = assets
        .dialog
        .textures
        .into_iter()
        .map(|texture| {
            texture.map(|texture| {
                let index = push(texture.clone(), Sampling::Linear);
                push(texture, Sampling::LinearRepeat);
                index
            })
        })
        .collect();
    TextureLayout {
        background: background.expect("a picture has tiles"),
        logo: logo.expect("a picture has tiles"),
        flash,
        dialog,
    }
}

fn texture_image(texture: &RgbaTexture, sampling: Sampling, dynamic: bool) -> Image {
    let usage = if dynamic {
        RenderAssetUsages::default()
    } else {
        RenderAssetUsages::RENDER_WORLD
    };
    let mut image = Image::new(
        Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        usage,
    );
    image.sampler = match sampling {
        Sampling::Nearest => ImageSampler::nearest(),
        Sampling::Linear => ImageSampler::linear(),
        Sampling::LinearRepeat => ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::linear()
        }),
    };
    image
}

fn sync_quads(
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut images: ResMut<Assets<Image>>,
    mut handles: ResMut<Textures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut slots: Query<(
        &QuadSlot,
        &Mesh2d,
        &MeshMaterial2d<ColorMaterial>,
        &mut Visibility,
    )>,
) {
    if !game.is_changed() {
        return;
    }
    while handles.0.len() < game.textures.len() {
        let index = handles.0.len();
        let image = texture_image(
            &game.textures[index],
            game.samplers[index],
            game.movie_texture == Some(index),
        );
        handles.0.push(images.add(image));
    }
    if let (Some(picture), Some(index)) = (game.picture.take(), game.movie_texture)
        && let Some(mut image) = images.get_mut(&handles.0[index])
    {
        image.data = Some(picture);
    }
    let size = |quad: &ScreenQuad| {
        quad.texture
            .and_then(|index| game.textures.get(index))
            .map(|texture| (texture.width, texture.height))
    };
    let mut existing = 0;
    for (slot, mesh, material, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        let Some(quad) = game.quads.get(slot.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            write_quad_mesh(&mut mesh, quad, size(quad));
        }
        let texture = quad.texture.map(|index| handles.0[index].clone());
        if materials
            .get(&material.0)
            .is_some_and(|current| current.texture != texture)
            && let Some(mut material) = materials.get_mut(&material.0)
        {
            material.texture = texture;
        }
        *visibility = Visibility::Visible;
    }
    for (index, quad) in game.quads.iter().enumerate().skip(existing) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        write_quad_mesh(&mut mesh, quad, size(quad));
        mesh.insert_indices(Indices::U16(vec![0, 1, 2, 1, 3, 2]));
        let material = ColorMaterial {
            color: Color::WHITE,
            alpha_mode: AlphaMode2d::Blend,
            texture: quad.texture.map(|index| handles.0[index].clone()),
            ..default()
        };
        commands.spawn((
            QuadSlot(index),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(material)),
            Transform::from_xyz(0.0, 0.0, index as f32 * 0.001),
            RenderLayers::layer(model_draw::DISPLAY_LAYER),
        ));
    }
}

/// GS modulation: a textured quad's vertex colour 0x80 multiplies by 1; a flat quad's colour is its own.
/// Alpha 0x80 is opaque either way.
fn write_quad_mesh(mesh: &mut Mesh, quad: &ScreenQuad, texture_size: Option<(u32, u32)>) {
    let positions: Vec<[f32; 3]> = quad
        .pt
        .iter()
        .map(|[x, y]| {
            [
                x - SCREEN_WIDTH as f32 / 2.0,
                SCREEN_HEIGHT as f32 / 2.0 - y,
                0.0,
            ]
        })
        .collect();
    let uvs: Vec<[f32; 2]> = match texture_size {
        Some((w, h)) => quad
            .uv
            .iter()
            .map(|[u, v]| [u / w as f32, v / h as f32])
            .collect(),
        None => vec![[0.0; 2]; 4],
    };
    let [r, g, b, a] = quad.rgba.map(f32::from);
    let scale = if texture_size.is_some() { 128.0 } else { 255.0 };
    let color = Color::srgba(r / scale, g / scale, b / scale, (a / 128.0).min(1.0));
    let color = LinearRgba::from(color).to_f32_array();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 4]);
}

/// ADXT output volumes changed by fade steps, and the pause of the voice and streamed-SE players.
fn apply_volumes(game: Res<Game>, mut sinks: Query<&mut AudioSink>) {
    for player in [game.bgm, game.map_bgm, game.voice[0], game.voice[1]]
        .into_iter()
        .flatten()
    {
        if let Ok(mut sink) = sinks.get_mut(player.entity) {
            let volume = sound::volume(player.level);
            if sink.volume() != volume {
                sink.set_volume(volume);
            }
        }
    }
    let se_voice = [
        game.voice[0],
        game.voice[1],
        game.stream_se[0],
        game.stream_se[1],
    ];
    for player in se_voice.into_iter().flatten() {
        if let Ok(sink) = sinks.get_mut(player.entity) {
            if game.se_voice_paused && !sink.is_paused() {
                sink.pause();
            } else if !game.se_voice_paused && sink.is_paused() {
                sink.play();
            }
        }
    }
}
