use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::WindowResolution;
use bt3_data::Iso9660;
use bt3_data::afs::{AfsArchive, partition_entry_for_id};
use bt3_data::bpe;
use bt3_data::gs::{RgbaTexture, decode_sprite_texture};
use bt3_data::tex::TextureResource;

const SCREEN_WIDTH: u32 = 512;
const SCREEN_HEIGHT: u32 = 448;
const TYPE_ZERO_FILE_ID: u32 = 0x3C1;
const FRAME_SECONDS: f32 = 1.0 / 30.0;

#[derive(Resource)]
struct ScreenImages(Vec<RgbaTexture>);

#[derive(Resource)]
struct LoadingState {
    frame_accumulator: f32,
    frame: u8,
    animation_timer: u16,
    cycle_count: u8,
    counter: u16,
    presses: u8,
}

impl Default for LoadingState {
    fn default() -> Self {
        Self {
            frame_accumulator: 0.0,
            frame: 0,
            animation_timer: 0,
            cycle_count: 3,
            counter: 0,
            presses: 1,
        }
    }
}

#[derive(Component)]
struct AnimatedBanner;

#[derive(Component)]
struct CounterPip {
    decade: u8,
    index: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso")
        });
    let textures = load_type_zero_textures(&image_path)?;

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Budokai Tenkaichi 3 - Loading Screen 0".to_owned(),
                        resolution: WindowResolution::new(SCREEN_WIDTH, SCREEN_HEIGHT)
                            .with_scale_factor_override(1.0),
                        resizable: false,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ScreenImages(textures))
        .init_resource::<LoadingState>()
        .add_systems(Startup, setup_screen)
        .add_systems(Update, update_loading_screen)
        .run();

    Ok(())
}

fn load_type_zero_textures(path: &PathBuf) -> Result<Vec<RgbaTexture>, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(path)?;
    let location = partition_entry_for_id(TYPE_ZERO_FILE_ID);
    let archive_path = format!("DATA/PZS3US{}.AFS", location.partition);
    let archive = AfsArchive::open(&mut image, archive_path)?;
    let packed = archive.read_entry(&mut image, location.index)?;
    let decoded = bpe::decode(&packed)?;
    let resource = TextureResource::parse(&decoded)?;
    resource
        .entries
        .iter()
        .map(decode_sprite_texture)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn setup_screen(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    screen_images: Res<ScreenImages>,
) {
    let handles = screen_images
        .0
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
        .collect::<Vec<_>>();

    commands.spawn(Camera2d);
    spawn_solid_sprite(
        &mut commands,
        Color::srgba_u8(0x22, 0x49, 0x68, 0x80),
        0.0,
        0.0,
        SCREEN_WIDTH as f32,
        SCREEN_HEIGHT as f32,
        0.0,
    );

    spawn_texture_sprite(
        &mut commands,
        handles[1].clone(),
        Rect::new(0.0, 0.0, 64.0, 128.0),
        [45.0, 265.0, 109.0, 393.0],
        1.0,
        Color::WHITE,
        Some(AnimatedBanner),
    );
    spawn_texture_sprite(
        &mut commands,
        handles[0].clone(),
        Rect::new(0.0, 21.0, 128.0, 128.0),
        [19.0, 318.0, 147.0, 425.0],
        2.0,
        Color::WHITE,
        None,
    );

    for decade in 0..=10u8 {
        let decade_x = if decade == 0 {
            0.0
        } else {
            30.0 + f32::from(decade - 1) * 25.0
        };
        let decade_y = if decade == 0 { 0.0 } else { 60.0 };
        for index in 0..10u8 {
            let (source_rect, base_rect) = if index == 0 {
                (
                    Rect::new(0.0, 0.0, 23.0, 17.0),
                    [113.0, 325.0, 136.0, 342.0],
                )
            } else {
                let top = 319.0 - f32::from(index - 1) * 6.0;
                (
                    Rect::new(32.0, 0.0, 55.0, 9.0),
                    [113.0, top, 136.0, top + 9.0],
                )
            };
            let sprite = spawn_texture_sprite(
                &mut commands,
                handles[0].clone(),
                source_rect,
                [
                    base_rect[0] + decade_x,
                    base_rect[1] + decade_y,
                    base_rect[2] + decade_x,
                    base_rect[3] + decade_y,
                ],
                3.0 + f32::from(decade) * 10.0 + f32::from(index) * 0.01,
                Color::WHITE,
                None,
            );
            commands.entity(sprite).insert(CounterPip { decade, index });
        }
    }
}

fn update_loading_screen(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LoadingState>,
    mut animated: Query<&mut Sprite, With<AnimatedBanner>>,
    mut pips: Query<(&CounterPip, &mut Visibility)>,
) {
    state.frame_accumulator += time.delta_secs();
    while state.frame_accumulator >= FRAME_SECONDS {
        state.frame_accumulator -= FRAME_SECONDS;
        if state.animation_timer > 12 || state.presses == 0 {
            state.presses = 1;
            state.frame ^= 1;
            state.animation_timer = 0;
            if state.frame == 0 {
                state.cycle_count -= 1;
                if state.cycle_count == 0 {
                    if state.counter < 100 {
                        state.counter += 1;
                    }
                    state.cycle_count = 3;
                }
            }
        }
        state.animation_timer += 1;
    }

    if let Ok(mut sprite) = animated.single_mut() {
        let u0 = f32::from(state.frame) * 64.0;
        sprite.rect = Some(Rect::new(u0, 0.0, u0 + 64.0, 128.0));
    }

    let visible_partial = (state.counter % 10) as u8;
    let full_decades = (state.counter / 10) as u8;
    for (pip, mut visibility) in &mut pips {
        let visible = if pip.decade == 0 {
            pip.index < visible_partial
        } else {
            pip.decade <= full_decades
        };
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    if keyboard.just_pressed(KeyCode::Space) || keyboard.just_pressed(KeyCode::Enter) {
        state.presses = 0;
    }
}

fn spawn_solid_sprite(
    commands: &mut Commands,
    color: Color,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    z: f32,
) -> Entity {
    commands
        .spawn((
            Sprite::from_color(color, Vec2::new(width, height)),
            Transform::from_xyz(x, y, z),
        ))
        .id()
}

fn spawn_texture_sprite(
    commands: &mut Commands,
    image: Handle<Image>,
    rect: Rect,
    screen_rect: [f32; 4],
    z: f32,
    color: Color,
    marker: Option<AnimatedBanner>,
) -> Entity {
    let [x0, y0, x1, y1] = screen_rect;
    let size = Vec2::new(x1 - x0, y1 - y0);
    let position = Vec3::new((x0 + x1) * 0.5 - 256.0, 224.0 - (y0 + y1) * 0.5, z as f32);
    let mut sprite = Sprite::from_image(image);
    sprite.custom_size = Some(size);
    sprite.rect = Some(rect);
    sprite.color = color;

    let entity = commands
        .spawn((sprite, Transform::from_translation(position)))
        .id();
    if let Some(marker) = marker {
        commands.entity(entity).insert(marker);
    }
    entity
}
