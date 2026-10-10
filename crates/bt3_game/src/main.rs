mod fade;
mod loading;
mod logo;
mod movie;
mod rng;

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::asset::RenderAssetUsages;
use bevy::audio::Volume;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::WindowResolution;
use bt3_data::Iso9660;
use bt3_data::adx;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::pack_section;
use bt3_data::tex::TextureResource;

use loading::{LoadingScreen, Quad};
use logo::LogoSequence;
use movie::{MoviePlayer, VideoStream};

const SCREEN_WIDTH: u32 = 512;
const SCREEN_HEIGHT: u32 = 448;
const FIRST_LOADING_FILE_ID: u32 = 0x3C1;
// gProgress->baseFile (0x1C1) + 0x19: menu archive 0, loaded by FirstRun_Main.
const LOGO_ARCHIVE_FILE_ID: u32 = 0x1C1 + 0x19;
// NTSC: one vertical blank is 1001/60000 s.
const VBLANK_SECONDS: f32 = 1001.0 / 60000.0;

enum Scene {
    Logos(LogoSequence),
    Movie(MoviePlayer, VideoStream),
    Black,
    Loading(LoadingScreen),
}

impl Scene {
    /// `Logo_Show` ends its frames with `Gfx_EndFrame(1)`; `Movie_Run` and `Load_RunBlocking` use 2.
    fn frame_seconds(&self) -> f32 {
        match self {
            Scene::Logos(_) | Scene::Black => VBLANK_SECONDS,
            Scene::Movie(..) | Scene::Loading(_) => 2.0 * VBLANK_SECONDS,
        }
    }
}

type Soundtrack = JoinHandle<std::io::Result<Vec<u8>>>;

#[derive(Resource)]
struct Game {
    scene: Scene,
    textures: Vec<RgbaTexture>,
    accumulator: f32,
    cross_latched: bool,
    start_latched: bool,
    quads: Vec<Quad>,
    iso_path: PathBuf,
    movie_texture: Option<usize>,
    soundtrack: Option<Soundtrack>,
    soundtrack_player: Option<Entity>,
    picture: Option<Vec<u8>>,
}

#[derive(Resource, Default)]
struct Textures(Vec<Handle<Image>>);

#[derive(Component)]
struct QuadSlot(usize);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image_path =
        PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
    let mut forced_type = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--screen" {
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
    let mut soundtrack = None;
    let (scene, textures, title) = match forced_type {
        Some(_) => {
            let screen = LoadingScreen::new(game_random, libc_random, None, forced_type);
            let textures = load_screen_textures(&mut image, screen.screen_type())?;
            let title = format!(
                "Budokai Tenkaichi 3 - Loading Screen {}",
                screen.screen_type()
            );
            (Scene::Loading(screen), textures, title)
        }
        None => {
            soundtrack = Some(decode_soundtrack(image_path.clone()));
            (
                Scene::Logos(LogoSequence::new()),
                load_logo_textures(&mut image)?,
                "Budokai Tenkaichi 3".to_owned(),
            )
        }
    };
    let movie_texture = soundtrack.is_some().then_some(textures.len());

    App::new()
        .insert_resource(ClearColor(Color::BLACK))
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
            accumulator: 0.0,
            cross_latched: false,
            start_latched: false,
            quads: Vec::new(),
            iso_path: image_path,
            movie_texture,
            soundtrack,
            soundtrack_player: None,
            picture: None,
        })
        .init_resource::<Textures>()
        .add_systems(Startup, setup)
        .add_systems(Update, (run_frames, sync_quads).chain())
        .run();

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

/// The opening soundtrack as an in-memory WAV, decoded while the logos play.
fn decode_soundtrack(iso_path: PathBuf) -> Soundtrack {
    std::thread::spawn(move || {
        let bytes = Iso9660::open(&iso_path)?.read_file(movie::ADX_PATH)?;
        let (header, pcm) = adx::decode(&bytes)?;
        Ok(adx::to_wav(
            header.sample_rate,
            header.channels as u16,
            &pcm,
        ))
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

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut handles: ResMut<Textures>,
    game: Res<Game>,
) {
    commands.spawn(Camera2d);
    handles.0 = game
        .textures
        .iter()
        .map(|texture| {
            let mut image = Image::new(
                Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                texture.pixels.clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = ImageSampler::nearest();
            images.add(image)
        })
        .collect();
    if game.movie_texture.is_some() {
        let size = Extent3d {
            width: movie::WIDTH,
            height: movie::HEIGHT,
            depth_or_array_layers: 1,
        };
        let mut picture = Image::new_fill(
            size,
            TextureDimension::D2,
            &[0, 0, 0, 0xFF],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        picture.sampler = ImageSampler::nearest();
        handles.0.push(images.add(picture));
    }
}

fn run_frames(
    mut commands: Commands,
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut game: ResMut<Game>,
) {
    // Pad 0: Space or the gamepad's south button is Cross (PADG_CROSS), Enter or Start is Start.
    if keyboard.just_pressed(KeyCode::Space)
        || gamepads
            .iter()
            .any(|pad| pad.just_pressed(GamepadButton::South))
    {
        game.cross_latched = true;
    }
    if keyboard.just_pressed(KeyCode::Enter)
        || gamepads
            .iter()
            .any(|pad| pad.just_pressed(GamepadButton::Start))
    {
        game.start_latched = true;
    }
    game.accumulator += time.delta_secs();
    while game.accumulator >= game.scene.frame_seconds() {
        game.accumulator -= game.scene.frame_seconds();
        let cross = std::mem::take(&mut game.cross_latched);
        let start = std::mem::take(&mut game.start_latched);
        let game = &mut *game;
        match &mut game.scene {
            Scene::Logos(logos) => {
                game.quads = logos.run_frame(cross || start);
                if logos.is_finished() {
                    start_opening_movie(game, &mut commands, &mut audio);
                }
            }
            Scene::Movie(player, video) => {
                let frame = player.run_frame(start, || video.next_picture());
                game.quads = frame.quads;
                if frame.picture.is_some() {
                    game.picture = frame.picture;
                }
                if player.is_finished() {
                    if let Some(entity) = game.soundtrack_player.take() {
                        commands.entity(entity).despawn();
                    }
                    game.scene = Scene::Black;
                }
            }
            Scene::Black => game.quads.clear(),
            Scene::Loading(screen) => game.quads = screen.run_frame([cross, false]),
        }
    }
}

/// `FirstRun_Main` after the logos: `Movie_PlayOpening` (partition 2 is already readable from the ISO).
fn start_opening_movie(game: &mut Game, commands: &mut Commands, audio: &mut Assets<AudioSource>) {
    let (Some(texture), Some(soundtrack)) = (game.movie_texture, game.soundtrack.take()) else {
        game.scene = Scene::Black;
        return;
    };
    let video = match VideoStream::open(&game.iso_path) {
        Ok(video) => video,
        Err(error) => {
            error!("cannot decode {} with ffmpeg: {error}", movie::PSS_PATH);
            game.scene = Scene::Black;
            return;
        }
    };
    match soundtrack.join() {
        Ok(Ok(wav)) => {
            let source = audio.add(AudioSource { bytes: wav.into() });
            let settings = PlaybackSettings::DESPAWN
                .with_volume(Volume::Decibels(movie::soundtrack_volume_db()));
            game.soundtrack_player = Some(commands.spawn((AudioPlayer(source), settings)).id());
        }
        Ok(Err(error)) => error!("cannot decode {}: {error}", movie::ADX_PATH),
        Err(_) => error!("soundtrack decoder thread panicked"),
    }
    game.scene = Scene::Movie(MoviePlayer::new(texture), video);
}

fn sync_quads(
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut images: ResMut<Assets<Image>>,
    handles: Res<Textures>,
    mut slots: Query<(&QuadSlot, &mut Sprite, &mut Transform, &mut Visibility)>,
) {
    if !game.is_changed() {
        return;
    }
    if let (Some(picture), Some(index)) = (game.picture.take(), game.movie_texture)
        && let Some(mut image) = images.get_mut(&handles.0[index])
    {
        image.data = Some(picture);
    }
    let mut existing = 0;
    for (slot, mut sprite, mut transform, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        match game.quads.get(slot.0) {
            Some(quad) => {
                (*sprite, *transform) = quad_sprite(quad, slot.0, &handles.0);
                *visibility = Visibility::Visible;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
    for (index, quad) in game.quads.iter().enumerate().skip(existing) {
        commands.spawn((QuadSlot(index), quad_sprite(quad, index, &handles.0)));
    }
}

/// GS vertex colour 0x80 modulates a texture by 1.0, and GS alpha 0x80 is fully opaque.
fn quad_sprite(quad: &Quad, order: usize, textures: &[Handle<Image>]) -> (Sprite, Transform) {
    let [x0, y0, x1, y1] = quad.screen.map(|v| v as f32);
    let [r, g, b, a] = quad.rgba.map(f32::from);
    let alpha = (a / 128.0).min(1.0);
    let mut sprite = match quad.texture.and_then(|index| textures.get(index)) {
        Some(image) => {
            let [u0, v0, u1, v1] = quad.uv.map(|v| v as f32);
            let mut sprite = Sprite::from_image(image.clone());
            sprite.rect = Some(Rect::new(u0, v0, u1, v1));
            sprite.color = Color::srgba(
                (r / 128.0).min(1.0),
                (g / 128.0).min(1.0),
                (b / 128.0).min(1.0),
                alpha,
            );
            sprite
        }
        None => Sprite::from_color(
            Color::srgba(r / 255.0, g / 255.0, b / 255.0, alpha),
            Vec2::ONE,
        ),
    };
    sprite.custom_size = Some(Vec2::new(x1 - x0, y1 - y0));
    let position = Vec3::new(
        (x0 + x1) * 0.5 - SCREEN_WIDTH as f32 / 2.0,
        SCREEN_HEIGHT as f32 / 2.0 - (y0 + y1) * 0.5,
        order as f32 * 0.001,
    );
    (sprite, Transform::from_translation(position))
}
