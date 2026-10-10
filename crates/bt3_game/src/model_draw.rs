//! Draws the battle models: each `ModelBatch` of the frame becomes a `Mesh2d` of screen-space triangles with
//! `GsModelMaterial`, which does what the GS does with program 0's two layers (see gs_model.wgsl). Z is the
//! GS depth (larger is nearer, GEQUAL like the 2D pipeline's depth test), kept below the 2D quads' layers.

use std::collections::HashMap;

use bevy::asset::{RenderAssetUsages, load_internal_asset, uuid_handle};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};
use bt3_data::gs::RgbaTexture;

use crate::battle::ModelBatch;
use crate::{Game, SCREEN_HEIGHT, SCREEN_WIDTH};

const GS_MODEL_SHADER: Handle<Shader> = uuid_handle!("6f0b8f0e-4a8e-4c1e-9d55-2b6a1c3e9a71");
/// GS Z is 24 bits.
const GS_Z_RANGE: f32 = 16_777_216.0;

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

pub fn add_to(app: &mut App) {
    load_internal_asset!(app, GS_MODEL_SHADER, "gs_model.wgsl", Shader::from_wgsl);
    app.add_plugins(Material2dPlugin::<GsModelMaterial>::default())
        .init_resource::<ModelTextures>();
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
        .map(|v| {
            [
                v.position[0] - SCREEN_WIDTH as f32 / 2.0,
                SCREEN_HEIGHT as f32 / 2.0 - v.position[1],
                -950.0 + v.position[2] / GS_Z_RANGE * 900.0,
            ]
        })
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
