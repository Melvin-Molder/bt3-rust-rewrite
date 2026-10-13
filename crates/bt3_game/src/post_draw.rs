//! Draws the post passes (`crate::post`): a chain of cameras runs one group of consecutive ops with the same source
//! and target each, in order, into the work buffers or back into the frame. Two chains: `StgFx_DrawPre`'s between
//! the stage camera and the fighters' camera, `StgFx_DrawPost`'s after the fighters and the effects; the HUD's
//! camera draws after them.

use std::collections::HashMap;

use bevy::asset::{RenderAssetUsages, load_internal_asset, uuid_handle};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    SpecializedMeshPipelineError, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPlugin};

use crate::Game;
use crate::model_draw::FrameBuffers;
use crate::post::{PostBuf, PostOp};

const GS_POST_SHADER: Handle<Shader> = uuid_handle!("5e2a9c41-7b3d-4f8a-9e16-c2d84a0f7b35");
/// The render layers of each chain's cameras (one each) and how many there are: a blur of three passes takes five,
/// a glow sixteen.
const POST_LAYER_BASE: [usize; 3] = [17, 5, 47];
const POST_CAMERAS: [usize; 3] = [28, 12, 16];
/// The camera orders of the chains: the pre chain after the stage camera (0); `BtlObj_BeginDraw`'s glow after the
/// fighters' camera (`model_draw::FIGHTER_ORDER`, 30), before the effects' (47); the post chain after that.
const POST_ORDER_BASE: [isize; 3] = [1, 48, 31];

/// One op's triangles: the source buffer, copied or blended (ALPHA 0x44), in the order of its group.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(GsPostKey)]
pub struct GsPostMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    /// The depth byte's plane and the op's mask (alpha by byte value), with x of `params` 1 when masked and y, z
    /// the plane's pixels per target pixel.
    #[texture(2)]
    plane: Handle<Image>,
    #[texture(3)]
    mask: Handle<Image>,
    #[uniform(4)]
    params: Vec4,
    /// The source texels read (x0, y0, x1, y1).
    #[uniform(5)]
    region: Vec4,
    blend: bool,
    add: bool,
    order: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GsPostKey {
    blend: bool,
    add: bool,
}

impl From<&GsPostMaterial> for GsPostKey {
    fn from(material: &GsPostMaterial) -> Self {
        Self {
            blend: material.blend,
            add: material.add,
        }
    }
}

impl Material2d for GsPostMaterial {
    fn fragment_shader() -> ShaderRef {
        GS_POST_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn depth_bias(&self) -> f32 {
        f32::from(self.order) * 0.001
    }

    /// A copy writes the texels as they are; a blended op takes the vertex alpha (the frame's alpha is left
    /// alone). No depth.
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let k = key.bind_group_data;
        let blend = (k.blend || k.add).then_some(BlendState {
            color: if k.add {
                BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                }
            } else {
                BlendComponent {
                    src_factor: BlendFactor::SrcAlpha,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                }
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        });
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = blend;
            }
        }
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The two work buffers (frame blocks 0x150 / 0x16C), and the 512-wide one over the same memory.
#[derive(Resource)]
pub struct WorkBuffers([Handle<Image>; 2], Handle<Image>);

/// The materials made so far, by source image, blend, order, mask image and target; the mask images, by their
/// alphas (made once, never changed: a material keeps the texture it was made with).
#[derive(Resource, Default)]
pub struct PostMaterials(
    HashMap<PostKey, Handle<GsPostMaterial>>,
    HashMap<Box<[u8; 256]>, Handle<Image>>,
);

type PostKey = (
    AssetId<Image>,
    bool,
    bool,
    u16,
    Option<AssetId<Image>>,
    PostBuf,
    Option<[u16; 4]>,
    bool,
);

/// A post camera: its chain, its place in the chain and the image it draws into.
#[derive(Component)]
pub struct PostCamera(usize, usize, Option<AssetId<Image>>);

/// A mesh of the post pass this frame.
#[derive(Component)]
pub struct PostSlot;

pub fn add_to(app: &mut App) {
    load_internal_asset!(app, GS_POST_SHADER, "gs_post.wgsl", Shader::from_wgsl);
    app.add_plugins(Material2dPlugin::<GsPostMaterial>::default())
        .init_resource::<PostMaterials>()
        .add_systems(Startup, spawn_post_cameras);
}

fn spawn_post_cameras(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let work: [Handle<Image>; 2] = std::array::from_fn(|_| {
        images.add(Image::new_target_texture(
            256,
            224,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ))
    });
    let wide = images.add(Image::new_target_texture(
        512,
        224,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    for chain in 0..3 {
        for i in 0..POST_CAMERAS[chain] {
            commands.spawn((
                Camera2d,
                Camera {
                    order: POST_ORDER_BASE[chain] + i as isize,
                    is_active: false,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                RenderTarget::from(work[0].clone()),
                RenderLayers::layer(POST_LAYER_BASE[chain] + i),
                PostCamera(chain, i, Some(work[0].id())),
            ));
        }
    }
    commands.insert_resource(WorkBuffers(work, wide));
}

/// The frame's ops, grouped (consecutive ops with the same source and target share a camera).
fn groups(ops: &[PostOp]) -> Vec<&[PostOp]> {
    let mut out: Vec<&[PostOp]> = Vec::new();
    let mut start = 0;
    for i in 1..=ops.len() {
        if i == ops.len()
            || (ops[i].source, ops[i].target) != (ops[start].source, ops[start].target)
        {
            if i > start {
                out.push(&ops[start..i]);
            }
            start = i;
        }
    }
    out
}

/// Runs after the frame logic and `sync_frame_buffers`: the cameras get their targets, the ops their meshes.
#[allow(clippy::too_many_arguments)]
pub fn sync_post(
    mut commands: Commands,
    game: Res<Game>,
    frames: Res<FrameBuffers>,
    work: Res<WorkBuffers>,
    mut cache: ResMut<PostMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GsPostMaterial>>,
    mut cameras: Query<(
        &mut PostCamera,
        &mut Camera,
        &mut RenderTarget,
        &mut Projection,
    )>,
    slots: Query<(Entity, &Mesh2d), With<PostSlot>>,
    plane: Res<crate::model_draw::DepthPlane>,
    ids: Res<crate::model_draw::IdPlane>,
    mut images: ResMut<Assets<Image>>,
) {
    if !game.is_changed() {
        return;
    }
    for (entity, mesh) in &slots {
        meshes.remove(&mesh.0);
        commands.entity(entity).despawn();
    }
    let image = |b: PostBuf| match b {
        PostBuf::Frame => frames.current(),
        PostBuf::Work(i) => work.0[i & 1].clone(),
        PostBuf::Wide => work.1.clone(),
    };
    let chains = [
        groups(&game.pre_ops),
        groups(&game.post_ops),
        groups(&game.obj_ops),
    ];
    for (mut cam, mut camera, mut target, mut projection) in &mut cameras {
        match chains[cam.0].get(cam.1) {
            Some(g) => {
                camera.is_active = true;
                let want = image(g[0].target);
                if cam.2 != Some(want.id()) {
                    cam.2 = Some(want.id());
                    *target = RenderTarget::from(want);
                    // `camera_system` sizes the projection again only on a changed projection (or image asset):
                    // a camera moved from a work buffer to the frame would keep the work buffer's size.
                    projection.set_changed();
                }
            }
            None => camera.is_active = false,
        }
    }
    for (chain, groups) in chains.iter().enumerate() {
        for (n, group) in groups.iter().enumerate().take(POST_CAMERAS[chain]) {
            for (k, op) in group.iter().enumerate() {
                let texture = image(op.source);
                // A masked op's alphas: their image (made the first time they are seen).
                let mask = op.mask.as_ref().map(|alphas| {
                    cache
                        .1
                        .entry(alphas.clone())
                        .or_insert_with(|| images.add(mask_image(alphas)))
                        .clone()
                });
                let key = (
                    texture.id(),
                    op.blend,
                    op.add,
                    k as u16,
                    mask.as_ref().map(Handle::id),
                    op.target,
                    op.region,
                    op.ids,
                );
                // The depth buffer's spare byte: the stage's depth, or the objects' ids once BtlObj_BeginDraw
                // copied them there.
                let byte = if op.ids { ids.image() } else { plane.image() };
                let [tw, th] = op.target.size();
                let material = cache
                    .0
                    .entry(key)
                    .or_insert_with(|| {
                        materials.add(GsPostMaterial {
                            texture,
                            plane: byte.clone(),
                            mask: mask.clone().unwrap_or_else(|| byte.clone()),
                            params: Vec4::new(
                                if mask.is_some() { 1.0 } else { 0.0 },
                                512.0 / tw,
                                448.0 / th,
                                0.0,
                            ),
                            region: {
                                let [sw, sh] = op.source.size();
                                op.region.map_or(Vec4::new(0.0, 0.0, sw, sh), |r| {
                                    Vec4::new(
                                        f32::from(r[0]),
                                        f32::from(r[1]),
                                        f32::from(r[2]),
                                        f32::from(r[3]),
                                    )
                                })
                            },
                            blend: op.blend || mask.is_some(),
                            add: op.add,
                            order: k as u16,
                        })
                    })
                    .clone();
                commands.spawn((
                    PostSlot,
                    Mesh2d(meshes.add(op_mesh(op))),
                    MeshMaterial2d(material),
                    Transform::default(),
                    RenderLayers::layer(POST_LAYER_BASE[chain] + n),
                ));
            }
        }
    }
}

/// A 256 x 1 table of alphas by byte value.
fn mask_image(alphas: &[u8; 256]) -> Image {
    let mut image = Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.data = Some(alphas.iter().flat_map(|&a| [0, 0, 0, a]).collect());
    image
}

/// The op's triangles in its target camera's space (pixels, y up, centred), texels normalised by the source.
fn op_mesh(op: &PostOp) -> Mesh {
    let [tw, th] = op.target.size();
    let [sw, sh] = op.source.size();
    let verts = || op.tris.iter().flatten();
    let positions: Vec<[f32; 3]> = verts()
        .map(|v| [v.xy[0] - tw / 2.0, th / 2.0 - v.xy[1], 0.0])
        .collect();
    let uvs: Vec<[f32; 2]> = verts().map(|v| [v.uv[0] / sw, v.uv[1] / sh]).collect();
    let colors: Vec<[f32; 4]> = verts()
        .map(|v| {
            let mut c = v.rgba.map(f32::from);
            if !op.blend {
                c[3] = 128.0;
            }
            c
        })
        .collect();
    let count = positions.len() as u32;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32((0..count).collect()));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_ops_with_one_source_and_target_share_a_camera() {
        let op = |s, t| PostOp {
            source: s,
            target: t,
            blend: false,
            mask: None,
            ids: false,
            add: false,
            region: None,
            tris: Vec::new(),
        };
        let ops = [
            op(PostBuf::Frame, PostBuf::Work(0)),
            op(PostBuf::Work(0), PostBuf::Work(1)),
            op(PostBuf::Work(0), PostBuf::Work(1)),
            op(PostBuf::Work(1), PostBuf::Frame),
        ];
        let g = groups(&ops);
        assert_eq!(g.iter().map(|g| g.len()).collect::<Vec<_>>(), [1, 2, 1]);
        assert!(groups(&[]).is_empty());
    }
}
