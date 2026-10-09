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
const FIRST_LOADING_FILE_ID: u32 = 0x3C1;
const FRAME_SECONDS: f32 = 1.0 / 30.0;
const LOAD_ITEM_COUNT: usize = 30;

#[derive(Clone, Copy, Debug, Default)]
struct LoadingItem {
    flags: u8,
    x: i32,
    y: i32,
}

#[derive(Resource)]
struct ScreenImages(Vec<RgbaTexture>);

#[derive(Resource)]
struct LoadingState {
    screen_type: u8,
    frame_accumulator: f32,
    frame: u8,
    animation_timer: u16,
    period: u16,
    cycle_count: u8,
    counter: u16,
    presses: i16,
    pulse_size: f32,
    pulse_angle: f32,
    target: usize,
    items: [LoadingItem; LOAD_ITEM_COUNT],
    taken: bool,
    pressed: bool,
}

impl LoadingState {
    fn new(screen_type: u8) -> Self {
        let (period, cycle_count, presses) = match screen_type {
            0 => (12, 3, 1),
            1 => (0, 1, 2),
            2 => (12, 5, 5),
            _ => (0, 1, 0),
        };
        let mut state = Self {
            screen_type,
            frame_accumulator: 0.0,
            frame: 0,
            animation_timer: 0,
            period,
            cycle_count,
            counter: 0,
            presses,
            pulse_size: 0.0,
            pulse_angle: 0.0,
            target: 0,
            items: [LoadingItem::default(); LOAD_ITEM_COUNT],
            taken: false,
            pressed: false,
        };
        if screen_type == 2 {
            state.reset_screen_2_items();
        }
        state
    }

    fn reset_screen_2_items(&mut self) {
        for (index, item) in self.items.iter_mut().enumerate() {
            item.flags = 0;
            item.x = 16 + ((index * 19 + 7) % 0x19C) as i32;
            item.y = 48 + ((index * 17 + 11) % 0x130) as i32;
        }
        self.target = 0;
        self.taken = false;
        self.pressed = false;
    }
}

#[derive(Component)]
struct AnimatedTexture {
    images: [Handle<Image>; 2],
    rects: [Rect; 2],
}

#[derive(Component)]
struct CounterDigit {
    position: usize,
    base_x: f32,
}

#[derive(Component)]
struct PulseSprite {
    base_size: Vec2,
}

#[derive(Component)]
struct CounterPip {
    decade: u8,
    index: u8,
}

#[cfg(test)]
mod tests {
    use super::LoadingState;

    #[test]
    fn screen_zero_counts_up_after_three_cycles() {
        let mut state = LoadingState::new(0);
        state.period = 12;
        state.presses = 1;
        state.animation_timer = 13;
        state.cycle_count = 1;
        state.frame = 1;

        super::advance_loading_state(&mut state, false);

        assert_eq!(state.frame, 0);
        assert_eq!(state.counter, 1);
    }

    #[test]
    fn screen_two_collects_target_item_and_resets_press_timer() {
        let mut state = LoadingState::new(2);
        state.target = 2;
        state.items[2].flags = 0;
        state.presses = 0;
        state.frame = 1;
        state.taken = false;
        state.pressed = false;

        super::advance_loading_state(&mut state, true);

        assert_eq!(state.items[2].flags & 1, 1);
        assert_eq!(state.presses, 4);
        assert!(state.taken);
        assert!(state.pressed);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image_path =
        PathBuf::from("rom").join("Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
    let mut screen_type = 0u8;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--screen" {
            let value = arguments
                .next()
                .ok_or("--screen requires a value of 0, 1, or 2")?;
            screen_type = value
                .to_string_lossy()
                .parse::<u8>()
                .map_err(|_| "--screen requires a value of 0, 1, or 2")?;
        } else {
            image_path = PathBuf::from(argument);
        }
    }
    if screen_type > 2 {
        return Err("--screen requires a value of 0, 1, or 2".into());
    }

    let textures = load_screen_textures(&image_path, screen_type)?;
    let title = format!("Budokai Tenkaichi 3 - Loading Screen {screen_type}");

    App::new()
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
        .insert_resource(ScreenImages(textures))
        .insert_resource(LoadingState::new(screen_type))
        .add_systems(Startup, setup_screen)
        .add_systems(Update, update_loading_screen)
        .run();

    Ok(())
}

fn load_screen_textures(
    path: &PathBuf,
    screen_type: u8,
) -> Result<Vec<RgbaTexture>, Box<dyn std::error::Error>> {
    let mut image = Iso9660::open(path)?;
    let location = partition_entry_for_id(FIRST_LOADING_FILE_ID + u32::from(screen_type));
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
    state: Res<LoadingState>,
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
        if state.screen_type == 0 {
            Color::srgb_u8(0x22, 0x49, 0x68)
        } else {
            Color::srgb_u8(0x46, 0x10, 0x12)
        },
        0.0,
        0.0,
        SCREEN_WIDTH as f32,
        SCREEN_HEIGHT as f32,
        0.0,
    );

    if state.screen_type == 0 {
        setup_counter_screen(&mut commands, &handles);
    } else {
        setup_pulse_screen(&mut commands, &handles);
    }
}

fn setup_counter_screen(commands: &mut Commands, handles: &[Handle<Image>]) {
    let banner = handles[1].clone();
    let sprite = spawn_texture_sprite(
        commands,
        banner.clone(),
        Rect::new(0.0, 0.0, 64.0, 128.0),
        [45.0, 265.0, 109.0, 393.0],
        1.0,
        Color::WHITE,
    );
    commands.entity(sprite).insert(AnimatedTexture {
        images: [banner.clone(), banner],
        rects: [
            Rect::new(0.0, 0.0, 64.0, 128.0),
            Rect::new(64.0, 0.0, 128.0, 128.0),
        ],
    });

    spawn_texture_sprite(
        commands,
        handles[0].clone(),
        Rect::new(0.0, 21.0, 128.0, 128.0),
        [19.0, 318.0, 147.0, 425.0],
        2.0,
        Color::WHITE,
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
                commands,
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
            );
            commands.entity(sprite).insert(CounterPip { decade, index });
        }
    }
}

fn setup_pulse_screen(commands: &mut Commands, handles: &[Handle<Image>]) {
    let first_image = handles[0].clone();
    let second_image = handles[1].clone();
    let main = spawn_texture_sprite(
        commands,
        first_image.clone(),
        Rect::new(0.0, 0.0, 128.0, 128.0),
        [26.0, 297.0, 154.0, 425.0],
        1.0,
        Color::WHITE,
    );
    commands.entity(main).insert(AnimatedTexture {
        images: [first_image, second_image],
        rects: [
            Rect::new(0.0, 0.0, 128.0, 128.0),
            Rect::new(0.0, 0.0, 128.0, 128.0),
        ],
    });

    let pulse = spawn_texture_sprite(
        commands,
        handles[2].clone(),
        Rect::new(0.0, 64.0, 67.0, 127.0),
        [121.0, 321.0, 188.0, 384.0],
        2.0,
        Color::WHITE,
    );
    commands.entity(pulse).insert(PulseSprite {
        base_size: Vec2::new(67.0, 63.0),
    });

    for position in 0..3usize {
        let x = 139.0 + position as f32 * 12.0;
        let digit = spawn_texture_sprite(
            commands,
            handles[2].clone(),
            Rect::new(32.0, 28.0, 48.0, 56.0),
            [x, 337.0, x + 16.0, 365.0],
            3.0 + position as f32 * 0.01,
            Color::WHITE,
        );
        commands.entity(digit).insert((
            CounterDigit {
                position,
                base_x: x,
            },
            Visibility::Hidden,
        ));
    }
}

fn update_loading_screen(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LoadingState>,
    mut animated: Query<
        (&AnimatedTexture, &mut Sprite),
        (Without<PulseSprite>, Without<CounterDigit>),
    >,
    mut pips: Query<(&CounterPip, &mut Visibility), Without<CounterDigit>>,
    mut digits: Query<
        (&CounterDigit, &mut Sprite, &mut Transform, &mut Visibility),
        (
            Without<CounterPip>,
            Without<PulseSprite>,
            Without<AnimatedTexture>,
        ),
    >,
    mut pulses: Query<
        (&PulseSprite, &mut Sprite),
        (Without<AnimatedTexture>, Without<CounterDigit>),
    >,
) {
    let pressed = keyboard.just_pressed(KeyCode::Space) || keyboard.just_pressed(KeyCode::Enter);
    state.frame_accumulator += time.delta_secs();
    while state.frame_accumulator >= FRAME_SECONDS {
        state.frame_accumulator -= FRAME_SECONDS;
        advance_loading_state(&mut state, pressed);
    }

    for (animation, mut sprite) in &mut animated {
        sprite.image = animation.images[usize::from(state.frame)].clone();
        sprite.rect = Some(animation.rects[usize::from(state.frame)]);
    }

    if state.screen_type == 0 {
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
    } else {
        let glyphs = decimal_glyphs(state.counter);
        let count_digits = if state.counter >= 100 {
            3
        } else if state.counter >= 10 {
            2
        } else {
            1
        };
        let offset = match count_digits {
            1 => -14.0,
            2 => -7.0,
            _ => 0.0,
        };
        for (digit, mut sprite, mut transform, mut visibility) in &mut digits {
            let value = glyphs[digit.position];
            transform.translation.x = digit.base_x + offset - 256.0;
            if state.counter != 0 && value < 10 {
                let u = f32::from(value & 7) * 16.0;
                let v = f32::from(value >> 3) * 28.0;
                sprite.rect = Some(Rect::new(u, v, u + 16.0, v + 28.0));
                *visibility = Visibility::Visible;
            } else {
                *visibility = Visibility::Hidden;
            }
        }
        for (pulse, mut sprite) in &mut pulses {
            sprite.custom_size = Some(Vec2::new(
                pulse.base_size.x + state.pulse_size * 2.0,
                pulse.base_size.y + state.pulse_size * 2.0,
            ));
        }
    }
}

fn advance_loading_state(state: &mut LoadingState, input_pressed: bool) {
    match state.screen_type {
        0 => {
            if state.period < state.animation_timer || state.presses == 0 {
                state.presses = 1;
                state.frame ^= 1;
                state.animation_timer = 0;
                if state.frame == 0 {
                    state.cycle_count -= 1;
                    if state.cycle_count == 0 {
                        state.counter = (state.counter + 1).min(100);
                        state.cycle_count = 3;
                    }
                }
            }
        }
        1 => {
            if state.presses == 0 {
                state.frame ^= 1;
                state.presses = i16::from(state.frame) + 2;
                if state.frame == 0 {
                    state.cycle_count -= 1;
                    if state.cycle_count == 0 {
                        state.counter = (state.counter + 1).min(999);
                        state.cycle_count = 1;
                    }
                }
            }
            state.pulse_size += state.pulse_angle.sin() * 0.5;
            state.pulse_angle += 0.2;
            if state.pulse_angle >= std::f32::consts::PI {
                state.pulse_angle -= std::f32::consts::TAU;
            }
        }
        2 => {
            if state.period < state.animation_timer || state.pressed {
                state.animation_timer = 0;
                state.frame ^= 1;
                if state.taken {
                    state.target = (state.target + 1) % LOAD_ITEM_COUNT;
                    while state.items[state.target].flags & 1 != 0 {
                        state.target = (state.target + 1) % LOAD_ITEM_COUNT;
                    }
                    state.taken = false;
                }
            }
            if state.presses == 0 {
                if state.frame == 1 {
                    state.items[state.target].flags |= 1;
                    state.presses = 5;
                    state.taken = true;
                    state.counter += 1;
                }
            }
        }
        _ => {}
    }

    if input_pressed {
        state.pressed = true;
        state.presses = state.presses.saturating_sub(1);
        if state.presses < 0 {
            state.presses = 0;
        }
    } else {
        state.pressed = false;
    }

    state.animation_timer += 1;
}

fn decimal_glyphs(value: u16) -> [u8; 3] {
    if value == 0 {
        [10, 10, 0]
    } else if value < 10 {
        [10, 10, value as u8]
    } else if value < 100 {
        [10, (value / 10) as u8, (value % 10) as u8]
    } else {
        [
            (value / 100) as u8,
            ((value / 10) % 10) as u8,
            (value % 10) as u8,
        ]
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
) -> Entity {
    let [x0, y0, x1, y1] = screen_rect;
    let size = Vec2::new(x1 - x0, y1 - y0);
    let position = Vec3::new((x0 + x1) * 0.5 - 256.0, 224.0 - (y0 + y1) * 0.5, z as f32);
    let mut sprite = Sprite::from_image(image);
    sprite.custom_size = Some(size);
    sprite.rect = Some(rect);
    sprite.color = color;

    commands
        .spawn((sprite, Transform::from_translation(position)))
        .id()
}
