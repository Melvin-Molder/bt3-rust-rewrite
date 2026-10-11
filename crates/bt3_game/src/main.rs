mod anim;
mod battle;
mod battle_seq;
mod btl_act;
mod btl_ai;
mod btl_blast;
mod btl_chr;
mod btl_flags;
mod btl_hit;
mod btl_input;
mod btl_menu;
mod btl_mgr;
mod btl_obj;
mod char_select;
mod dialog;
mod draw;
mod duel_menu;
mod fade;
mod flash;
mod font;
mod hud;
mod hud_gauge;
mod hud_gs;
mod loading;
mod logo;
mod main_menu;
mod mcflow;
mod menu_res;
mod model_draw;
mod movie;
mod rng;
mod save;
mod se;
mod sound;
mod stage_draw;
mod title;
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
    Logos(LogoSequence),
    Movie(MoviePlayer, VideoStream),
    Title(Box<TitleScreen>),
    MainMenu(Box<MainMenuScreen>),
    DuelMenu(Box<DuelMenuScreen>),
    CharSelect(Box<CharSelScreen>),
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
    /// `Logo_Show`, `Title_Run` and `MainMenu_Run` end their frames with `Gfx_EndFrame(1)`; `Movie_Run` and
    /// `Load_RunBlocking` use 2.
    fn frame_seconds(&self) -> f32 {
        match self {
            Scene::Logos(_)
            | Scene::Title(_)
            | Scene::MainMenu(_)
            | Scene::DuelMenu(_)
            | Scene::CharSelect(_)
            | Scene::Black => VBLANK_SECONDS,
            Scene::Movie(..) | Scene::Loading(_) | Scene::BattleLoad(_) | Scene::Battle(_) => {
                2.0 * VBLANK_SECONDS
            }
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
    /// `PAD_*` bits pressed since the last battle frame.
    battle_latch: u32,
    quads: Vec<ScreenQuad>,
    iso_path: PathBuf,
    random: GameRandom,
    /// newlib `rand()`, behind `Rand_IntRange`.
    libc_random: LibcRandom,
    /// The loading screen type shown last (`Load_InitScreen` avoids repeating it).
    last_loading_type: Option<u16>,
    /// The battle's model textures (material, shading rows) and this frame's model triangles.
    model_textures: (Vec<RgbaTexture>, Vec<RgbaTexture>),
    model_batches: Vec<battle::ModelBatch>,
    /// The stage texture file and this frame's stage triangles.
    stage_textures: Vec<RgbaTexture>,
    stage_batches: Vec<stage_draw::StageBatch>,
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
    demo_count: u32,
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
    char_select: Option<CharSelData>,
    /// The character select's fields of `gProgress`.
    char_progress: CharSelProgress,
    /// `ChrTbl_WrapCostume`'s table (common file 4): costumes per character id.
    costumes: Vec<i32>,
    /// Portraits and stage pictures by file: their first texture.
    pictures: HashMap<u32, usize>,
    save: save::SaveData,
    /// Menu music by file, once decoded, and the file being decoded.
    bgm_sources: HashMap<u32, Handle<AudioSource>>,
    pending_bgm: Option<(u32, Decoding)>,
    /// `gProgress->flags & MPROG_SAVE_LOADED`: set once the title was left for the main menu.
    save_loaded: bool,
    /// `gProgress->mainMenuItem`.
    main_menu_item: i32,
    bgm: Option<Player>,
    voice: Option<Player>,
    /// Streamed-SE player 0 (`StreamSe_PlayDefault(0, id)`) and the file being decoded for it.
    stream_se: Option<Player>,
    pending_stream_se: Option<Decoding>,
    /// The voice entity was spawned this update, so it is not in the world yet.
    voice_fresh: bool,
    /// The last voice played to its end (`ADXT_STAT_PLAYEND`).
    voice_ended: bool,
    pending_voice: Option<Decoding>,
    picture: Option<Vec<u8>>,
    /// `--title`: start at Progress_Main mode 1 instead of FirstRun_Main.
    skip_to_title: bool,
    /// `--menu`: start at Progress_Main mode 4.
    skip_to_menu: bool,
    /// `--battle`: start straight at `Battle_Main` with characters 0 and 1 on stage 0.
    skip_to_battle: bool,
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
    let mut skip_to_battle = false;
    let mut log_fps = false;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--title" {
            skip_to_title = true;
        } else if argument == "--menu" {
            skip_to_title = true;
            skip_to_menu = true;
        } else if argument == "--fps" {
            log_fps = true;
        } else if argument == "--battle" {
            skip_to_title = true;
            skip_to_battle = true;
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
    let timer = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
    let (game_random, libc_random) = rng::init_from_timer(timer);
    let mut image = Iso9660::open(&image_path)?;
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
    let samplers = vec![Sampling::Nearest; textures.len()];
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
            battle_latch: 0,
            quads: Vec::new(),
            iso_path: image_path,
            random,
            libc_random,
            last_loading_type: None,
            model_textures: (Vec::new(), Vec::new()),
            model_batches: Vec::new(),
            stage_textures: Vec::new(),
            stage_batches: Vec::new(),
            hud_textures: Vec::new(),
            hud_prims: Vec::new(),
            hud_layer: None,
            xfade_captures: 0,
            xfade_alpha: 0,
            hud_clamp_from: usize::MAX,
            demo_count: 0,
            movie_texture,
            soundtrack,
            soundtrack_source: None,
            music,
            music_source: None,
            title: None,
            main_menu: None,
            duel_menu: None,
            duel: DuelProgress::default(),
            char_select: None,
            char_progress: CharSelProgress::default(),
            costumes: Vec::new(),
            pictures: HashMap::new(),
            save: save::SaveData::default(),
            bgm_sources: HashMap::new(),
            pending_bgm: None,
            save_loaded: false,
            main_menu_item: 0,
            bgm: None,
            voice: None,
            stream_se: None,
            pending_stream_se: None,
            voice_fresh: false,
            voice_ended: false,
            pending_voice: None,
            picture: None,
            skip_to_title,
            skip_to_menu,
            skip_to_battle,
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
                sync_quads,
                model_draw::sync_models,
                model_draw::sync_stage,
                model_draw::sync_hud,
                model_draw::sync_frame_buffers,
                apply_volumes,
            )
                .chain(),
        );
    model_draw::add_to(&mut app);
    if log_fps {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
    app.run();

    Ok(())
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
    Ok(font::Font::new(slots, line_height))
}

/// `Title_Init`: section 1 of gMenuArc1, BPE-packed.
fn load_title_assets(iso_path: &Path) -> Result<TitleAssets, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(iso_path)?;
    let archive = read_file(&mut image, title::ARCHIVE_FILE_ID)?;
    let res = bpe::decode(pack_section(&archive, title::ARCHIVE_SECTION)?)?;
    TitleAssets::parse(&res)
}

fn adx_to_wav(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (header, pcm) = adx::decode(bytes).map_err(|error| error.to_string())?;
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

/// Pad 0 game buttons: arrows or the d-pad, Space or South (Cross), Escape or North (Triangle), Enter or Start.
const BUTTONS: [(KeyCode, GamepadButton, u32); 7] = [
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
const BATTLE_BUTTONS: [(GamepadButton, u32); 15] = [
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
    (GamepadButton::LeftThumb, btl_input::pad::L3),
    (GamepadButton::RightThumb, btl_input::pad::R3),
];

/// `Voice_GetStat(0)`: 2 while the file is decoded, 3 while it sounds, 5 once it played to its end, else 0.
fn voice_stat(game: &mut Game, players: &Query<(), With<AudioPlayer>>) -> i32 {
    if game.pending_voice.is_some() {
        return 2;
    }
    match game.voice {
        Some(player) if game.voice_fresh || players.contains(player.entity) => 3,
        Some(_) => {
            game.voice = None;
            game.voice_ended = true;
            anim::VOICE_PLAYEND
        }
        None if game.voice_ended => anim::VOICE_PLAYEND,
        None => 0,
    }
}

/// `Adx_StopAll`.
fn stop_all(game: &mut Game, commands: &mut Commands) {
    stop_player(&mut game.bgm, commands);
    stop_player(&mut game.voice, commands);
    stop_player(&mut game.stream_se, commands);
    game.pending_voice = None;
    game.pending_stream_se = None;
    game.pending_bgm = None;
    game.voice_ended = false;
}

/// What a menu frame asked of the sound players.
fn play_sounds(game: &mut Game, commands: &mut Commands, sounds: Vec<TitleSound>) {
    for sound in sounds {
        match sound {
            TitleSound::Voice(file_id) => {
                stop_player(&mut game.voice, commands);
                game.voice_ended = false;
                game.pending_voice = Some(decode_adx_file(game.iso_path.clone(), file_id));
            }
            TitleSound::VoiceStop => {
                stop_player(&mut game.voice, commands);
                game.pending_voice = None;
                game.voice_ended = false;
            }
            TitleSound::FadeOutStep => {
                for player in [&mut game.voice, &mut game.bgm].into_iter().flatten() {
                    player.level = sound::fade_out_step(player.level);
                }
            }
            TitleSound::Bgm(file_id) => {
                stop_player(&mut game.bgm, commands);
                game.pending_bgm = None;
                play_bgm(game, commands, file_id);
            }
            TitleSound::BgmFadeOutStep => {
                if let Some(player) = &mut game.bgm {
                    player.level = sound::fade_out_step(player.level);
                }
            }
            TitleSound::Effect(bank, id) => game.se.play_se(bank, id),
            TitleSound::StreamSe(file_id) => {
                stop_player(&mut game.stream_se, commands);
                game.pending_stream_se = Some(decode_adx_file(game.iso_path.clone(), file_id));
            }
        }
    }
}

fn run_frames(
    mut commands: Commands,
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    players: Query<(), With<AudioPlayer>>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut game: ResMut<Game>,
) {
    // Pad 0 for the battle: `Pad.held` with a latch that keeps a press shorter than a frame for one frame, and the
    // left stick as `Pad_Update` reads it (y down).
    let mut battle_held = 0;
    for (key, bit) in BATTLE_KEYS {
        if keyboard.just_pressed(key) {
            game.battle_latch |= bit;
        }
        if keyboard.pressed(key) {
            battle_held |= bit;
        }
    }
    let mut battle_stick = [0.0f32; 2];
    for pad in gamepads.iter() {
        for (button, bit) in BATTLE_BUTTONS {
            if pad.just_pressed(button) {
                game.battle_latch |= bit;
            }
            if pad.pressed(button) {
                battle_held |= bit;
            }
        }
        if let (Some(x), Some(y)) = (
            pad.get(GamepadAxis::LeftStickX),
            pad.get(GamepadAxis::LeftStickY),
        ) {
            let raw = |v: f32| (v * 127.5 + 127.5).clamp(0.0, 255.0) as u8;
            let s = btl_input::stick_from_raw(raw(x), raw(-y));
            if s != [0.0; 2] {
                battle_stick = s;
            }
        }
    }
    let mut held = 0;
    for (key, button, bit) in BUTTONS {
        if keyboard.just_pressed(key) || gamepads.iter().any(|pad| pad.just_pressed(button)) {
            game.pressed_latch |= bit;
        }
        if keyboard.pressed(key) || gamepads.iter().any(|pad| pad.pressed(button)) {
            held |= bit;
        }
    }
    game.voice_fresh = false;
    start_pending_voice(&mut game, &mut commands, &mut audio);
    start_pending_stream_se(&mut game, &mut commands, &mut audio);
    start_pending_bgm(&mut game, &mut commands, &mut audio);
    let pad2 = gamepads.iter().count() >= 2;
    if std::mem::take(&mut game.skip_to_title) {
        if game.skip_to_battle {
            // 1P vs COM: pad 0 against the computer.
            let mut setup = BattleSetup::default();
            setup.sides[1].chara = 1;
            setup.sides[1].control = 2;
            setup.sides[1].pad = 1;
            start_battle(&mut game, setup);
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
        let cross = pressed & title::PAD_CROSS != 0;
        let start = pressed & title::PAD_START != 0;
        // Snd_Update at the top of every frame loop sends what the last frame queued.
        game.se.update();
        let game = &mut *game;
        let stat = voice_stat(game, &players);
        match &mut game.scene {
            Scene::Logos(logos) => {
                game.quads = into_screen(logos.run_frame(cross || start));
                if logos.is_finished() {
                    start_opening_movie(game, &mut commands, &mut audio);
                }
            }
            Scene::Movie(player, video) => {
                let frame = player.run_frame(start, || video.next_picture());
                game.quads = into_screen(frame.quads);
                if frame.picture.is_some() {
                    game.picture = frame.picture;
                }
                if player.is_finished() {
                    // The soundtrack plays on the BGM player.
                    stop_player(&mut game.bgm, &mut commands);
                    start_title(game, &mut commands, &mut audio);
                }
            }
            Scene::Title(screen) => {
                let frame = screen.run_frame(
                    held,
                    pressed,
                    &mut game.demo_count,
                    &mut game.random,
                    &mut game.font,
                );
                let result = screen.result();
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
                        TitleResult::Demo => {
                            info!("the attract battle is not ported yet");
                            game.scene = Scene::Black;
                        }
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
                        MainMenuResult::Mode(38) => start_duel_menu(game, &mut commands),
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
                        DuelMenuResult::Select(mode) => {
                            info!("progress mode {mode} (team select) is not ported yet");
                            game.scene = Scene::Black;
                        }
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
                    [(pressed, held), (0, 0)],
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
                    // Duel_Main: Adx_StopAll after CharSel_Run.
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
                let pad = btl_input::PadState {
                    held: battle_held | std::mem::take(&mut game.battle_latch),
                    left: battle_stick,
                    status: 0,
                };
                let pads = [pad, btl_input::PadState::default()];
                let frame = battle.run_frame(
                    start,
                    pads,
                    stat,
                    &mut game.libc_random,
                    &mut game.random,
                    Some(&mut game.font),
                    shown,
                );
                game.model_batches = frame.models;
                game.stage_batches = frame.stage;
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
                play_sounds(game, &mut commands, frame.sounds);
                if let Some(reason) = frame.exit {
                    leave_battle(game, &mut commands, &mut audio, reason);
                }
            }
        }
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
) {
    stop_all(game, commands);
    game.model_batches.clear();
    game.stage_batches.clear();
    game.hud_prims.clear();
    game.hud_layer = None;
    game.xfade_alpha = 0;
    game.hud_clamp_from = usize::MAX;
    if reason & 8 != 0 {
        start_main_menu(game, commands, audio);
    } else if reason & 0x10 != 0 && game.duel.battle_type == 0 {
        start_char_select(game, commands);
    } else if reason & 0x80 != 0 {
        start_duel_menu(game, commands);
    } else {
        info!("the screen after the battle (reason {reason:#X}) is not ported yet");
        game.scene = Scene::Black;
    }
}

/// `Battle_Main` from `Battle_Init`: `Battle_Load` shows the loading screen (it takes over the shared generators)
/// while the files are read.
fn start_battle(game: &mut Game, setup: BattleSetup) {
    info!("battle setup {setup:?}");
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
    let job = std::thread::spawn(move || {
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
    game.stage_textures = assets.stage_textures.clone();
    game.hud_textures = assets.hud_textures.clone();
    game.hud_clamp_from = assets.menu_sheet.base;
    game.scene = Scene::Battle(Box::new(battle::Battle::new(assets)));
    play_bgm(game, commands, battle::BGM_FIRST + setup.bgm.max(0) as u32);
}

fn into_screen(quads: Vec<loading::Quad>) -> Vec<ScreenQuad> {
    quads.into_iter().map(ScreenQuad::from).collect()
}

fn stop_player(player: &mut Option<Player>, commands: &mut Commands) {
    if let Some(player) = player.take() {
        commands.entity(player.entity).despawn();
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

/// `StreamSe_PlayDefault(0, id)` (volume 0x40) once the file is decoded.
fn start_pending_stream_se(
    game: &mut Game,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
) {
    if !game
        .pending_stream_se
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        return;
    }
    let mut source = None;
    finish_decoding(
        &mut game.pending_stream_se,
        &mut source,
        audio,
        "a streamed sound",
    );
    if let Some(source) = source {
        let level = sound::out_volume(sound::CH_SE0, 0x40);
        let settings = PlaybackSettings::DESPAWN.with_volume(sound::volume(level));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.stream_se = Some(Player { entity, level });
    }
}

/// `Voice_Play(0, id, 0x80, 0)` once the file is decoded.
fn start_pending_voice(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    if !game
        .pending_voice
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        return;
    }
    let mut source = None;
    finish_decoding(&mut game.pending_voice, &mut source, audio, "a title voice");
    if let Some(source) = source {
        let level = sound::out_volume(sound::CH_VOICE0, 0x80);
        let settings = PlaybackSettings::DESPAWN.with_volume(sound::volume(level));
        let entity = commands.spawn((AudioPlayer(source), settings)).id();
        game.voice = Some(Player { entity, level });
        game.voice_fresh = true;
    }
}

/// `Movie_PlayOpening` (partition 2 is already readable from the ISO).
fn start_opening_movie(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    let Some(texture) = game.movie_texture else {
        game.scene = Scene::Black;
        return;
    };
    let video = match VideoStream::open(&game.iso_path) {
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
    game.scene = Scene::Movie(MoviePlayer::new(texture), video);
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
    let screen = match TitleScreen::new(data, game.save_loaded, &mut game.random) {
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
fn start_char_select(game: &mut Game, commands: &mut Commands) {
    if game.costumes.is_empty() {
        // Common file 4: word 1 is the byte offset of the 0x3C-byte character entries; +0xA the costume count.
        match Iso9660::open(&game.iso_path)
            .map_err(Into::into)
            .and_then(|mut image| read_file(&mut image, 4))
        {
            Ok(file) => {
                let base = file.get(4..8).map_or(0, |b| {
                    u32::from_le_bytes(b.try_into().unwrap()) as usize & !3
                });
                game.costumes = (0..0xA1)
                    .map(|id| {
                        let at = base + id * 0x3C + 0xA;
                        file.get(at..at + 2)
                            .map_or(1, |b| i32::from(u16::from_le_bytes([b[0], b[1]])))
                    })
                    .collect();
            }
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
                return;
            }
        }
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
        game.costumes.clone(),
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

/// ADXT output volumes changed by fade steps.
fn apply_volumes(game: Res<Game>, mut sinks: Query<&mut AudioSink>) {
    for player in [game.bgm, game.voice].into_iter().flatten() {
        if let Ok(mut sink) = sinks.get_mut(player.entity) {
            let volume = sound::volume(player.level);
            if sink.volume() != volume {
                sink.set_volume(volume);
            }
        }
    }
}
