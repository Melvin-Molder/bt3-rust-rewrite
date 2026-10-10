//! Draws the battle's 3D passes: each `ModelBatch` (fighters, VU1 program 0) and `StageBatch` (stage, program 4)
//! of the frame becomes a `Mesh2d` of screen-space triangles. Z is the GS depth (larger is nearer, GEQUAL like the
//! 2D pipeline's depth test). Everything goes through the sorted 2D pass so `Battle_Draw`'s order holds: the
//! stage groups in order, then the fighters, then the 2D quads; each material sets the depth writes and blending
//! of its GS state.

use std::collections::HashMap;

use bevy::asset::{RenderAssetUsages, load_internal_asset, uuid_handle};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};
use bt3_data::gs::RgbaTexture;

use crate::battle::ModelBatch;
use crate::stage_draw::StageBatch;
use crate::{Game, SCREEN_HEIGHT, SCREEN_WIDTH};

const GS_MODEL_SHADER: Handle<Shader> = uuid_handle!("6f0b8f0e-4a8e-4c1e-9d55-2b6a1c3e9a71");
const GS_STAGE_SHADER: Handle<Shader> = uuid_handle!("2b7f4c1d-8e3a-4f6b-a5c9-1d0e7f3b6a24");
/// GS Z is 24 bits.
const GS_Z_RANGE: f32 = 16_777_216.0;
/// Sort positions in the 2D pass: stage groups from -900 (100 apart), the fighters after them.
const STAGE_SORT: f32 = -900.0;
const FIGHTER_SORT: f32 = -100.0;

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
        set_pipeline_state(
            descriptor,
            Some(BlendState::ALPHA_BLENDING),
            matches!(group, 1 | 2),
        );
        Ok(())
    }
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
    images: Vec<Handle<Image>>,
    materials: HashMap<(usize, usize, usize), Handle<GsStageMaterial>>,
}

#[derive(Component)]
pub struct StageSlot(usize);

pub fn add_to(app: &mut App) {
    load_internal_asset!(app, GS_MODEL_SHADER, "gs_model.wgsl", Shader::from_wgsl);
    load_internal_asset!(app, GS_STAGE_SHADER, "gs_stage.wgsl", Shader::from_wgsl);
    app.add_plugins((
        Material2dPlugin::<GsModelMaterial>::default(),
        Material2dPlugin::<GsStageMaterial>::default(),
    ))
    .init_resource::<ModelTextures>()
    .init_resource::<StageTextures>();
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
    if game.stage_textures.len() < textures.images.len() {
        *textures = StageTextures::default();
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
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, stq);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
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
    let count = positions.len() as u32;
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32((0..count).collect()));
}
