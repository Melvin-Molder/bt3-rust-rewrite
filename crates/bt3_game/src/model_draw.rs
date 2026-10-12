//! Draws the battle's 3D passes: each `ModelBatch` (fighters, VU1 program 0) and `StageBatch` (stage, program 4)
//! of the frame becomes a `Mesh2d` of screen-space triangles. Z is the GS depth (larger is nearer, GEQUAL like the
//! 2D pipeline's depth test). Everything goes through the sorted 2D pass so `Battle_Draw`'s order holds: the
//! stage groups in order, then the fighters, then the 2D quads; each material sets the depth writes and blending
//! of its GS state.

use std::collections::HashMap;

use bevy::asset::{RenderAssetUsages, load_internal_asset, uuid_handle};
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d,
    RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use bt3_data::gs::RgbaTexture;

use crate::battle::ModelBatch;
use crate::eft::{EffectBlend, EffectTri};
use crate::hud::{HudBlend, HudPrim};
use crate::stage_draw::StageBatch;
use crate::{Game, SCREEN_HEIGHT, SCREEN_WIDTH};

const GS_MODEL_SHADER: Handle<Shader> = uuid_handle!("6f0b8f0e-4a8e-4c1e-9d55-2b6a1c3e9a71");
const GS_STAGE_SHADER: Handle<Shader> = uuid_handle!("2b7f4c1d-8e3a-4f6b-a5c9-1d0e7f3b6a24");
const GS_HUD_SHADER: Handle<Shader> = uuid_handle!("9c3e5a71-0d2b-4e8f-b6a4-7f15c2d8e903");
const GS_EFFECT_SHADER: Handle<Shader> = uuid_handle!("d41c7e2a-5b93-4f06-8e1d-3a7c9b2f6e58");
/// GS Z is 24 bits.
const GS_Z_RANGE: f32 = 16_777_216.0;
/// Sort positions in the 2D pass: stage groups from -900 (100 apart), the fighters after them.
const STAGE_SORT: f32 = -900.0;
const FIGHTER_SORT: f32 = -100.0;
/// The HUD comes after the fighters (`BtlGame_Draw`), in the order of its sprites.
const HUD_SORT: f32 = 100.0;
/// The effects' sorted pass (`Ot_Draw`) between the fighters and the HUD, in the order of its batches.
const EFFECT_SORT: f32 = 0.0;

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct GsModelMaterial {
    #[texture(0)]
    #[sampler(1)]
    base: Handle<Image>,
    #[texture(2)]
    #[sampler(3)]
    shade: Handle<Image>,
}

impl Material2d for GsModelMaterial {
    fn fragment_shader() -> ShaderRef {
        GS_MODEL_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn depth_bias(&self) -> f32 {
        FIGHTER_SORT
    }

    /// The fighters' layer 0 does not blend (ALPHA_1 0x64 takes the source) and writes depth.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        set_pipeline_state(descriptor, None, true);
        Ok(())
    }
}

/// One stage material in one draw group.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GsStageKey)]
pub struct GsStageMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    #[uniform(2)]
    params: Vec4,
    group: u8,
    order: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GsStageKey {
    group: u8,
}

impl From<&GsStageMaterial> for GsStageKey {
    fn from(material: &GsStageMaterial) -> Self {
        Self {
            group: material.group,
        }
    }
}

impl Material2d for GsStageMaterial {
    fn fragment_shader() -> ShaderRef {
        GS_STAGE_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn depth_bias(&self) -> f32 {
        STAGE_SORT + f32::from(self.group) * 100.0 + f32::from(self.order) * 0.01
    }

    /// `StgModel_Draw`: ALPHA_1 0x44 everywhere; group 0 and the blended groups 3..5 do not write depth.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let group = key.bind_group_data.group;
        // The debris (group 6) writes depth too: TEST_1 0x51809 keeps the depth of a fading piece (alpha below
        // 0x80) unwritten, which is not reproduced.
        set_pipeline_state(
            descriptor,
            Some(BlendState::ALPHA_BLENDING),
            matches!(group, 1 | 2 | 6),
        );
        Ok(())
    }
}

/// One HUD sprite's texture, mask and blend.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GsHudKey)]
pub struct GsHudMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    #[texture(2)]
    #[sampler(3)]
    mask: Handle<Image>,
    #[uniform(4)]
    params: Vec4,
    blend: HudMaterialBlend,
    order: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum HudMaterialBlend {
    Alpha,
    Add,
    /// The CPU-drawn gauge layer: P + (1 - a) * background.
    Layer,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GsHudKey {
    blend: HudMaterialBlend,
}

impl From<&GsHudMaterial> for GsHudKey {
    fn from(material: &GsHudMaterial) -> Self {
        Self {
            blend: material.blend,
        }
    }
}

impl Material2d for GsHudMaterial {
    fn fragment_shader() -> ShaderRef {
        GS_HUD_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn depth_bias(&self) -> f32 {
        if self.blend == HudMaterialBlend::Layer {
            return HUD_SORT - 1.0;
        }
        HUD_SORT + f32::from(self.order) * 0.01
    }

    /// `HudGfx_SetEnv`: ALPHA 0x44, no depth writes; ALPHA 0x58 adds and leaves the alpha plane alone.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let keep_alpha = BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        let colour = |dst_factor| BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor,
            operation: BlendOperation::Add,
        };
        let blend = match key.bind_group_data.blend {
            HudMaterialBlend::Alpha => BlendState::ALPHA_BLENDING,
            HudMaterialBlend::Add => BlendState {
                color: colour(BlendFactor::One),
                alpha: keep_alpha,
            },
            HudMaterialBlend::Layer => BlendState {
                color: colour(BlendFactor::OneMinusSrcAlpha),
                alpha: keep_alpha,
            },
        };
        set_pipeline_state(descriptor, Some(blend), false);
        Ok(())
    }
}

/// A run of effect triangles with one texture and one blend equation.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GsEffectKey)]
pub struct GsEffectMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    #[uniform(2)]
    params: Vec4,
    blend: EffectBlend,
    zwrite: bool,
    order: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GsEffectKey {
    blend: EffectBlend,
    zwrite: bool,
}

impl From<&GsEffectMaterial> for GsEffectKey {
    fn from(material: &GsEffectMaterial) -> Self {
        Self {
            blend: material.blend,
            zwrite: material.zwrite,
        }
    }
}

impl Material2d for GsEffectMaterial {
    fn fragment_shader() -> ShaderRef {
        GS_EFFECT_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn depth_bias(&self) -> f32 {
        EFFECT_SORT + f32::from(self.order) * 0.001
    }

    /// `Ot_AddEnv`: depth test without writes (but for the triangles that write it); the layer's ALPHA_1 /
    /// ALPHA_2. The frame's alpha is left alone.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let keep_alpha = BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        let colour = |src_factor, dst_factor, operation| BlendComponent {
            src_factor,
            dst_factor,
            operation,
        };
        let color = match key.bind_group_data.blend {
            EffectBlend::Alpha => colour(
                BlendFactor::SrcAlpha,
                BlendFactor::OneMinusSrcAlpha,
                BlendOperation::Add,
            ),
            EffectBlend::Subtract => colour(
                BlendFactor::One,
                BlendFactor::One,
                BlendOperation::ReverseSubtract,
            ),
            EffectBlend::Add => colour(BlendFactor::One, BlendFactor::One, BlendOperation::Add),
            EffectBlend::AddDest => colour(BlendFactor::Dst, BlendFactor::One, BlendOperation::Add),
        };
        set_pipeline_state(
            descriptor,
            Some(BlendState {
                color,
                alpha: keep_alpha,
            }),
            key.bind_group_data.zwrite,
        );
        Ok(())
    }
}

/// The battle's effect textures as uploaded so far, a blank one for untextured triangles, and the materials.
#[derive(Resource, Default)]
pub struct EffectTextures {
    /// Each texture clamped (CLAMP 5) and wrapping (the sorted pass's CLAMP 0).
    images: Vec<[Handle<Image>; 2]>,
    blank: Option<Handle<Image>>,
    materials: HashMap<(Option<usize>, bool, EffectBlend, bool, u8, u16), Handle<GsEffectMaterial>>,
}

#[derive(Component)]
pub struct EffectSlot(usize);

/// One mesh per run of triangles that share a texture and a blend, in the order of the sorted pass.
pub fn sync_effects(
    mut commands: Commands,
    game: Res<Game>,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<EffectTextures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GsEffectMaterial>>,
    mut slots: Query<(
        &EffectSlot,
        &Mesh2d,
        &mut MeshMaterial2d<GsEffectMaterial>,
        &mut Visibility,
    )>,
) {
    if !game.is_changed() {
        return;
    }
    if game.effect_textures.len() < textures.images.len() {
        *textures = EffectTextures::default();
    }
    while textures.images.len() < game.effect_textures.len() {
        let texture = &game.effect_textures[textures.images.len()];
        // TEX1 0x60 bilinear.
        let clamped = images.add(model_image(texture, ImageAddressMode::ClampToEdge));
        let wrapped = images.add(model_image(texture, ImageAddressMode::Repeat));
        textures.images.push([clamped, wrapped]);
    }
    let blank = textures
        .blank
        .get_or_insert_with(|| {
            images.add(model_image(
                &RgbaTexture {
                    width: 1,
                    height: 1,
                    pixels: vec![0xFF; 4],
                },
                ImageAddressMode::ClampToEdge,
            ))
        })
        .clone();
    type Batch<'a> = (
        Option<usize>,
        bool,
        EffectBlend,
        bool,
        u8,
        Vec<&'a EffectTri>,
    );
    let mut batches: Vec<Batch> = Vec::new();
    for tri in &game.effects {
        match batches.last_mut() {
            Some((t, c, b, z, a, list))
                if *t == tri.texture
                    && *c == tri.clamp
                    && *b == tri.blend
                    && *z == tri.zwrite
                    && *a == tri.alpha_ref =>
            {
                list.push(tri)
            }
            _ => batches.push((
                tri.texture,
                tri.clamp,
                tri.blend,
                tri.zwrite,
                tri.alpha_ref,
                vec![tri],
            )),
        }
    }
    let mut material = |index: usize,
                        texture: Option<usize>,
                        clamp: bool,
                        blend: EffectBlend,
                        zwrite: bool,
                        alpha_ref: u8,
                        textures: &mut EffectTextures| {
        let key = (texture, clamp, blend, zwrite, alpha_ref, index as u16);
        if let Some(handle) = textures.materials.get(&key) {
            return handle.clone();
        }
        let image = texture
            .and_then(|i| textures.images.get(i))
            .map(|pair| pair[usize::from(!clamp)].clone())
            .unwrap_or_else(|| blank.clone());
        let handle = materials.add(GsEffectMaterial {
            texture: image,
            params: Vec4::new(
                if texture.is_some() { 1.0 } else { 0.0 },
                if blend == EffectBlend::AddDest {
                    1.0
                } else {
                    0.0
                },
                if matches!(blend, EffectBlend::Add | EffectBlend::Subtract) {
                    1.0
                } else {
                    0.0
                },
                f32::from(alpha_ref),
            ),
            blend,
            zwrite,
            order: index as u16,
        });
        textures.materials.insert(key, handle.clone());
        handle
    };
    let mut existing = 0;
    for (slot, mesh, mut handle, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        let Some((texture, clamp, blend, zwrite, alpha_ref, tris)) = batches.get(slot.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            write_effect_mesh(&mut mesh, tris);
        }
        let wanted = material(
            slot.0,
            *texture,
            *clamp,
            *blend,
            *zwrite,
            *alpha_ref,
            &mut textures,
        );
        if handle.0 != wanted {
            handle.0 = wanted;
        }
        *visibility = Visibility::Visible;
    }
    for (index, (texture, clamp, blend, zwrite, alpha_ref, tris)) in
        batches.iter().enumerate().skip(existing)
    {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        write_effect_mesh(&mut mesh, tris);
        commands.spawn((
            EffectSlot(index),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(material(
                index,
                *texture,
                *clamp,
                *blend,
                *zwrite,
                *alpha_ref,
                &mut textures,
            )),
            Transform::default(),
        ));
    }
}

/// Bevy's mesh allocator skips a mesh without vertices but still copies its data, logging a "use-after-free" error
/// each frame; an empty batch is given one degenerate triangle (three equal points, nothing drawn) instead.
fn non_empty<T: Copy + Default>(mut v: Vec<T>) -> Vec<T> {
    if v.is_empty() {
        v = vec![T::default(); 3];
    }
    v
}

fn write_effect_mesh(mesh: &mut Mesh, tris: &[&EffectTri]) {
    let vertices = || tris.iter().flat_map(|t| t.vertices.iter());
    let positions: Vec<[f32; 3]> = vertices().map(|v| screen_position(v.position)).collect();
    let stq: Vec<[f32; 4]> = vertices()
        .map(|v| [v.stq[0], v.stq[1], v.stq[2], 0.0])
        .collect();
    let colors: Vec<[f32; 4]> = vertices().map(|v| v.rgba.map(f32::from)).collect();
    let (positions, stq, colors) = (non_empty(positions), non_empty(stq), non_empty(colors));
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, stq);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32((0..count).collect()));
}

fn set_pipeline_state(
    descriptor: &mut RenderPipelineDescriptor,
    blend: Option<BlendState>,
    depth_write: bool,
) {
    if let Some(fragment) = &mut descriptor.fragment {
        for target in fragment.targets.iter_mut().flatten() {
            target.blend = blend;
        }
    }
    if let Some(depth) = &mut descriptor.depth_stencil {
        depth.depth_write_enabled = Some(depth_write);
    }
}

/// `StgModel_SetTestOpaque` (alpha >= 0x7F) for group 2, `StgModel_SetTestDefault` (alpha >= 1) for groups 0 and
/// 1; the blended groups have no alpha test.
fn stage_alpha_ref(group: usize) -> f32 {
    match group {
        2 => 127.0 / 128.0,
        0 | 1 => 1.0 / 128.0,
        _ => 0.0,
    }
}

/// The battle's model textures as uploaded so far, and the materials made of them.
#[derive(Resource, Default)]
pub struct ModelTextures {
    base: Vec<Handle<Image>>,
    shade: Vec<Handle<Image>>,
    materials: HashMap<[usize; 2], Handle<GsModelMaterial>>,
}

#[derive(Component)]
pub struct ModelSlot(usize);

/// The stage textures of the battle and the materials made of them by (group, order, texture).
#[derive(Resource, Default)]
pub struct StageTextures {
    /// The stage revision the images are of.
    rev: u32,
    images: Vec<Handle<Image>>,
    materials: HashMap<(usize, usize, usize), Handle<GsStageMaterial>>,
}

#[derive(Component)]
pub struct StageSlot(usize);

pub fn add_to(app: &mut App) {
    load_internal_asset!(app, GS_MODEL_SHADER, "gs_model.wgsl", Shader::from_wgsl);
    load_internal_asset!(app, GS_STAGE_SHADER, "gs_stage.wgsl", Shader::from_wgsl);
    load_internal_asset!(app, GS_HUD_SHADER, "gs_hud.wgsl", Shader::from_wgsl);
    load_internal_asset!(app, GS_EFFECT_SHADER, "gs_effect.wgsl", Shader::from_wgsl);
    app.add_plugins((
        Material2dPlugin::<GsModelMaterial>::default(),
        Material2dPlugin::<GsStageMaterial>::default(),
        Material2dPlugin::<GsHudMaterial>::default(),
        Material2dPlugin::<GsEffectMaterial>::default(),
    ))
    .init_resource::<ModelTextures>()
    .init_resource::<StageTextures>()
    .init_resource::<HudTextures>()
    .init_resource::<EffectTextures>()
    .add_systems(Startup, spawn_cameras);
}

/// The 2D layer shown on the window: the frame buffers and the quads.
pub const DISPLAY_LAYER: usize = 1;

/// The two frame buffers the 3D passes draw into: one is drawn to, the other keeps the frame `ScrXfade_PostDraw`
/// captured last.
#[derive(Resource)]
pub struct FrameBuffers {
    images: [Handle<Image>; 2],
    current: usize,
    captures: u32,
    swap: bool,
}

#[derive(Component)]
pub struct SceneCamera;

#[derive(Component)]
pub struct LiveFrame;

#[derive(Component)]
pub struct KeptFrame;

fn spawn_cameras(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let images: [Handle<Image>; 2] = std::array::from_fn(|_| {
        images.add(Image::new_target_texture(
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ))
    });
    commands.spawn((Camera2d, SceneCamera, RenderTarget::from(images[0].clone())));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        RenderLayers::layer(DISPLAY_LAYER),
    ));
    commands.spawn((
        Sprite::from_image(images[0].clone()),
        Transform::from_xyz(0.0, 0.0, -990.0),
        RenderLayers::layer(DISPLAY_LAYER),
        LiveFrame,
    ));
    commands.spawn((
        Sprite::from_image(images[1].clone()),
        Transform::from_xyz(0.0, 0.0, -980.0),
        RenderLayers::layer(DISPLAY_LAYER),
        Visibility::Hidden,
        KeptFrame,
    ));
    commands.insert_resource(FrameBuffers {
        images,
        current: 0,
        captures: 0,
        swap: false,
    });
}

/// `ScrXfade`: a captured frame stays in its buffer and is drawn over the following frames at `alpha / 128`
/// while the 3D passes go on in the other buffer.
pub fn sync_frame_buffers(
    game: Res<Game>,
    mut buffers: ResMut<FrameBuffers>,
    mut camera: Query<&mut RenderTarget, With<SceneCamera>>,
    mut live: Query<&mut Sprite, (With<LiveFrame>, Without<KeptFrame>)>,
    mut kept: Query<(&mut Sprite, &mut Visibility), With<KeptFrame>>,
) {
    // The frame captured last update was drawn into the current buffer: keep it, draw on in the other.
    if std::mem::take(&mut buffers.swap) {
        buffers.current ^= 1;
        let image = buffers.images[buffers.current].clone();
        for mut target in &mut camera {
            *target = RenderTarget::from(image.clone());
        }
        for mut sprite in &mut live {
            sprite.image = image.clone();
        }
    }
    if game.xfade_captures != buffers.captures {
        buffers.captures = game.xfade_captures;
        buffers.swap = true;
        let image = buffers.images[buffers.current].clone();
        for (mut sprite, _) in &mut kept {
            sprite.image = image.clone();
        }
    }
    for (mut sprite, mut visibility) in &mut kept {
        let alpha = game.xfade_alpha.clamp(0, 128) as f32 / 128.0;
        sprite.color = Color::WHITE.with_alpha(alpha);
        *visibility = if alpha > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Runs after the frame logic, like `sync_quads`.
pub fn sync_stage(
    mut commands: Commands,
    game: Res<Game>,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<StageTextures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GsStageMaterial>>,
    mut slots: Query<(
        &StageSlot,
        &Mesh2d,
        &mut MeshMaterial2d<GsStageMaterial>,
        &mut Visibility,
    )>,
) {
    if !game.is_changed() {
        return;
    }
    if game.stage_textures.len() < textures.images.len() || textures.rev != game.stage_rev {
        *textures = StageTextures {
            rev: game.stage_rev,
            ..StageTextures::default()
        };
    }
    while textures.images.len() < game.stage_textures.len() {
        let texture = &game.stage_textures[textures.images.len()];
        // CLAMP_1 0: wrapping.
        textures
            .images
            .push(images.add(model_image(texture, ImageAddressMode::Repeat)));
    }
    let mut material = |batch: &StageBatch, textures: &mut StageTextures| {
        let key = (batch.group, batch.order, batch.texture);
        if let Some(handle) = textures.materials.get(&key) {
            return handle.clone();
        }
        let handle = materials.add(GsStageMaterial {
            texture: textures.images[batch.texture].clone(),
            params: Vec4::new(stage_alpha_ref(batch.group), 0.0, 0.0, 0.0),
            group: batch.group as u8,
            order: batch.order as u16,
        });
        textures.materials.insert(key, handle.clone());
        handle
    };
    let mut existing = 0;
    for (slot, mesh, mut handle, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        let Some(batch) = game.stage_batches.get(slot.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            write_stage_mesh(&mut mesh, batch);
        }
        let wanted = material(batch, &mut textures);
        if handle.0 != wanted {
            handle.0 = wanted;
        }
        *visibility = Visibility::Visible;
    }
    for (index, batch) in game.stage_batches.iter().enumerate().skip(existing) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        write_stage_mesh(&mut mesh, batch);
        commands.spawn((
            StageSlot(index),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(material(batch, &mut textures)),
            Transform::default(),
        ));
    }
}

fn write_stage_mesh(mesh: &mut Mesh, batch: &StageBatch) {
    let positions: Vec<[f32; 3]> = batch
        .vertices
        .iter()
        .map(|v| screen_position(v.position))
        .collect();
    let stq: Vec<[f32; 4]> = batch
        .vertices
        .iter()
        .map(|v| [v.stq[0], v.stq[1], v.stq[2], 0.0])
        .collect();
    let colors: Vec<[f32; 4]> = batch.vertices.iter().map(|v| v.rgba).collect();
    let (positions, stq, colors) = (non_empty(positions), non_empty(stq), non_empty(colors));
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, stq);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32((0..count).collect()));
}

/// The battle's HUD sheets as uploaded so far, a blank texture for untextured sprites, and the materials.
#[derive(Resource, Default)]
pub struct HudTextures {
    images: Vec<Handle<Image>>,
    blank: Option<Handle<Image>>,
    materials: HashMap<(Option<usize>, HudBlend, u8, u16), Handle<GsHudMaterial>>,
}

#[derive(Component)]
pub struct HudSlot(usize);

/// The CPU-drawn gauge layer and its image.
#[derive(Component)]
pub struct HudLayerSlot(Handle<Image>);

/// Runs after the frame logic, like `sync_quads`: the gauge layer, then one mesh per notice sprite, in the 3D
/// pass's frame buffer.
#[allow(clippy::too_many_arguments)]
pub fn sync_hud(
    mut commands: Commands,
    game: Res<Game>,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<HudTextures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GsHudMaterial>>,
    mut slots: Query<(
        &HudSlot,
        &Mesh2d,
        &mut MeshMaterial2d<GsHudMaterial>,
        &mut Visibility,
    )>,
    mut layers: Query<(&HudLayerSlot, &mut Visibility), Without<HudSlot>>,
) {
    if !game.is_changed() {
        return;
    }
    sync_hud_layer(
        &mut commands,
        game.hud_layer.as_deref(),
        &mut images,
        &mut meshes,
        &mut materials,
        &mut layers,
    );
    if game.hud_textures.len() < textures.images.len() {
        textures.images.clear();
        textures.materials.clear();
    }
    while textures.images.len() < game.hud_textures.len() {
        let index = textures.images.len();
        let texture = &game.hud_textures[index];
        // TEX1 0x60 bilinear; CLAMP 0 wrapping, the menu sheet under the default environment's clamp.
        let address = if index >= game.hud_clamp_from {
            ImageAddressMode::ClampToEdge
        } else {
            ImageAddressMode::Repeat
        };
        textures
            .images
            .push(images.add(model_image(texture, address)));
    }
    let blank = textures
        .blank
        .get_or_insert_with(|| {
            images.add(model_image(
                &RgbaTexture {
                    width: 1,
                    height: 1,
                    pixels: vec![0xFF; 4],
                },
                ImageAddressMode::ClampToEdge,
            ))
        })
        .clone();
    let mut material = |index: usize, prim: &HudPrim, textures: &mut HudTextures| {
        let key = (prim.texture, prim.blend, prim.rgba[3], index as u16);
        if let Some(handle) = textures.materials.get(&key) {
            return handle.clone();
        }
        let image = |i: Option<usize>| {
            i.and_then(|i| textures.images.get(i).cloned())
                .unwrap_or_else(|| blank.clone())
        };
        let (mask, masked) = match prim.blend {
            HudBlend::Alpha => (None, 0.0),
            HudBlend::AddMasked(mask) => (Some(mask), 1.0),
        };
        let handle = materials.add(GsHudMaterial {
            texture: image(prim.texture),
            mask: image(mask),
            params: Vec4::new(
                if prim.texture.is_some() { 1.0 } else { 0.0 },
                masked,
                f32::from(prim.rgba[3]) / 128.0,
                0.0,
            ),
            blend: if mask.is_some() {
                HudMaterialBlend::Add
            } else {
                HudMaterialBlend::Alpha
            },
            order: index as u16,
        });
        textures.materials.insert(key, handle.clone());
        handle
    };
    let mut existing = 0;
    for (slot, mesh, mut handle, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        let Some(prim) = game.hud_prims.get(slot.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            write_hud_mesh(&mut mesh, prim);
        }
        let wanted = material(slot.0, prim, &mut textures);
        if handle.0 != wanted {
            handle.0 = wanted;
        }
        *visibility = Visibility::Visible;
    }
    for (index, prim) in game.hud_prims.iter().enumerate().skip(existing) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        write_hud_mesh(&mut mesh, prim);
        commands.spawn((
            HudSlot(index),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(material(index, prim, &mut textures)),
            Transform::default(),
        ));
    }
}

/// The gauge layer: one full-frame quad over a 512 x 448 image rewritten each frame.
fn sync_hud_layer(
    commands: &mut Commands,
    pixels: Option<&[u8]>,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GsHudMaterial>,
    layers: &mut Query<(&HudLayerSlot, &mut Visibility), Without<HudSlot>>,
) {
    if let Ok((slot, mut visibility)) = layers.single_mut() {
        match pixels {
            Some(pixels) => {
                if let Some(mut image) = images.get_mut(&slot.0) {
                    image.data = Some(pixels.to_vec());
                }
                *visibility = Visibility::Visible;
            }
            None => *visibility = Visibility::Hidden,
        }
        return;
    }
    let Some(pixels) = pixels else {
        return;
    };
    let mut image = Image::new(
        Extent3d {
            width: SCREEN_WIDTH,
            height: SCREEN_HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels.to_vec(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);
    let (w, h) = (SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, 0.0], [w, h], [0.0, h]];
    let positions: Vec<[f32; 3]> = corners
        .iter()
        .map(|&[x, y]| {
            let [x, y, _] = screen_position([x, y, 0.0]);
            [x, y, 0.0]
        })
        .collect();
    let uvs: Vec<[f32; 2]> = corners.iter().map(|&[x, y]| [x / w, y / h]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[0.0f32; 4]; 6]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[128.0f32; 4]; 6]);
    mesh.insert_indices(Indices::U32((0..6).collect()));
    let material = materials.add(GsHudMaterial {
        texture: handle.clone(),
        mask: handle.clone(),
        params: Vec4::new(0.0, 0.0, 0.0, 1.0),
        blend: HudMaterialBlend::Layer,
        order: 0,
    });
    commands.spawn((
        HudLayerSlot(handle),
        Mesh2d(meshes.add(mesh)),
        MeshMaterial2d(material),
        Transform::default(),
    ));
}

/// The GS samples a pixel at its corner, the 2D pass at its centre: half a pixel moves one onto the other.
fn write_hud_mesh(mesh: &mut Mesh, prim: &HudPrim) {
    let positions: Vec<[f32; 3]> = prim
        .vertices
        .iter()
        .map(|v| {
            let [x, y, _] = screen_position([v.pos[0] + 0.5, v.pos[1] + 0.5, 0.0]);
            [x, y, 0.0]
        })
        .collect();
    let uvs: Vec<[f32; 2]> = prim.vertices.iter().map(|v| v.uv).collect();
    let masks: Vec<[f32; 4]> = prim
        .vertices
        .iter()
        .map(|v| [v.mask_uv[0], v.mask_uv[1], 0.0, 0.0])
        .collect();
    let color = prim.rgba.map(f32::from);
    let (positions, uvs, masks) = (non_empty(positions), non_empty(uvs), non_empty(masks));
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, masks);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; count as usize]);
    mesh.insert_indices(Indices::U32((0..count).collect()));
}

/// GS screen pixels and Z to the 2D camera's space.
fn screen_position(p: [f32; 3]) -> [f32; 3] {
    [
        p[0] - SCREEN_WIDTH as f32 / 2.0,
        SCREEN_HEIGHT as f32 / 2.0 - p[1],
        -950.0 + p[2] / GS_Z_RANGE * 900.0,
    ]
}

/// Runs after the frame logic, like `sync_quads`.
pub fn sync_models(
    mut commands: Commands,
    game: Res<Game>,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<ModelTextures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GsModelMaterial>>,
    mut slots: Query<(
        &ModelSlot,
        &Mesh2d,
        &mut MeshMaterial2d<GsModelMaterial>,
        &mut Visibility,
    )>,
) {
    if !game.is_changed() {
        return;
    }
    if game.model_textures.0.len() < textures.base.len() {
        // A new battle: its textures start again from 0.
        *textures = ModelTextures::default();
    }
    while textures.base.len() < game.model_textures.0.len() {
        let texture = &game.model_textures.0[textures.base.len()];
        textures
            .base
            .push(images.add(model_image(texture, ImageAddressMode::Repeat)));
    }
    while textures.shade.len() < game.model_textures.1.len() {
        let texture = &game.model_textures.1[textures.shade.len()];
        textures
            .shade
            .push(images.add(model_image(texture, ImageAddressMode::ClampToEdge)));
    }
    let mut material = |key: [usize; 2], textures: &mut ModelTextures| {
        if let Some(handle) = textures.materials.get(&key) {
            return handle.clone();
        }
        let handle = materials.add(GsModelMaterial {
            base: textures.base[key[0]].clone(),
            shade: textures.shade[key[1]].clone(),
        });
        textures.materials.insert(key, handle.clone());
        handle
    };
    let mut existing = 0;
    for (slot, mesh, mut handle, mut visibility) in &mut slots {
        existing = existing.max(slot.0 + 1);
        let Some(batch) = game.model_batches.get(slot.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            write_batch_mesh(&mut mesh, batch);
        }
        let wanted = material(batch.textures, &mut textures);
        if handle.0 != wanted {
            handle.0 = wanted;
        }
        *visibility = Visibility::Visible;
    }
    for (index, batch) in game.model_batches.iter().enumerate().skip(existing) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        write_batch_mesh(&mut mesh, batch);
        commands.spawn((
            ModelSlot(index),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(material(batch.textures, &mut textures)),
            Transform::default(),
        ));
    }
}

/// Raw GS colours (not sRGB-decoded): the shader does the GS arithmetic on them. Bilinear (TEX1 0x60).
fn model_image(texture: &RgbaTexture, address: ImageAddressMode) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture.pixels.clone(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

fn write_batch_mesh(mesh: &mut Mesh, batch: &ModelBatch) {
    let positions: Vec<[f32; 3]> = batch
        .vertices
        .iter()
        .map(|v| screen_position(v.position))
        .collect();
    let uvs: Vec<[f32; 2]> = batch
        .vertices
        .iter()
        .map(|v| [v.stq[0], v.stq[1]])
        .collect();
    let colors: Vec<[f32; 4]> = batch
        .vertices
        .iter()
        .map(|v| [v.uq, 0.0, v.stq[2], 1.0])
        .collect();
    let (positions, uvs, colors) = (non_empty(positions), non_empty(uvs), non_empty(colors));
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32((0..count).collect()));
}
