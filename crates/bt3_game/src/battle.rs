//! `Battle_Main` (src/battle/battle.c) up to its first frames: `Battle_Load` reads the fighters' model and motion
//! files behind the loading screen, then each frame the fighters loop their idle motion and are drawn the way
//! `BtlObjDraw_DrawModel` and VU1 program 0 draw them, seen through side 0's fighter camera (`ChrCam_CalcLockOn`).
//!
//! Not ported yet: the fighters' actions (they stand at the stage's starting places), the hair / cloth chains,
//! the face animation, the stage itself, the intro camera and the HUD.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use bt3_data::gs::{
    RgbaTexture, decode_clut256_upload, decode_sprite_texture, psmt8_row_from_upload,
};
use bt3_data::model::{CharaModel, ModelPart, TEXTURE_SECTION, tex0_cbp, tex0_tbp};
use bt3_data::motion::{Motion, NODE_MAX};
use bt3_data::pack_section;
use bt3_data::stage::StageResource;
use bt3_data::tex::TextureResource;

use crate::char_select::BattleSetup;

/// `BTL_FILE_CHARA`: `0x590 + chara * 10 + costume`.
pub const CHARA_FILE: u32 = 0x590;
/// The character's motion table (`slot->file[1]`): `0x598 + chara * 10`.
const MOTION_FILE: u32 = 0x598;
/// `BtlAct_GetIdleMotion` on the ground at full health.
const IDLE_MOTION: usize = 0;
/// `BtlObj_GetDefaultAnimStep`: frames per update.
const ANIM_STEP: f32 = 2.0;
/// `BTL_FILE_STAGE`: `0x171 + stage`.
pub const STAGE_FILE: u32 = 0x171;
/// `Battle_ResetWork`: `Bgm_Play(Battle_GetBgm() + 0x10B16)`.
pub const BGM_FIRST: u32 = 0x10B16;
/// `BtlObjLight_Init`: section 1 of common file 2 is the light's texture file.
const COMMON_LIGHT_FILE: u32 = 2;
/// `ChrCam_KeepBehindHead` keeps the camera behind this node.
const HEAD_NODE: u16 = 0x30;
/// `BtlObjLight_Init`'s direction, used until the stage gives one.
const DEFAULT_LIGHT: [f32; 3] = [0.34, -0.006, 0.941];

/// The GS frame the battle draws into (`View_InitLayout`, full screen): XYOFFSET is (2048 - 256, 2048 - 224).
const OFFSET_X: f32 = 2048.0 - 256.0;
const OFFSET_Y: f32 = 2048.0 - 224.0;

/// A PS2 matrix: four rows, applied to row vectors (`v.x * m[0] + v.y * m[1] + v.z * m[2] + v.w * m[3]`).
type Mtx = [[f32; 4]; 4];

const IDENTITY: Mtx = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// What `Battle_Load` leaves for the draw: the fighters' models and their textures as GS colours.
pub struct BattleAssets {
    /// Material textures (bilinear, wrapping: TEX1 0x60, CLAMP_1 0).
    pub base_textures: Vec<RgbaTexture>,
    /// One 256 x 1 row per shading CLUT: the light ramp looked up through it (bilinear, clamped: CLAMP_2 5).
    pub shade_textures: Vec<RgbaTexture>,
    fighters: Vec<FighterModel>,
    stage: StageResource,
}

struct FighterModel {
    model: CharaModel,
    /// Per part, per batch: the material texture and the shading row.
    batch_textures: Vec<Vec<[usize; 2]>>,
    idle: Motion,
}

/// Reads what the battle draws so far. `read` returns a file of the AFS archives by id.
pub fn load(
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
    setup: &BattleSetup,
) -> Result<BattleAssets, String> {
    let error = |e: std::io::Error| e.to_string();
    // BtlObjLight_FindRes0 uploads texture 0 of the light file to block 0x3D40; the models' TEX0_2 reads that
    // block as a 256-wide PSMT8 texture (TBW 4) on its first row.
    let common = read(COMMON_LIGHT_FILE)?;
    let light = TextureResource::parse(pack_section(&common, 1).map_err(error)?).map_err(error)?;
    let ramp_entry = light
        .entries
        .first()
        .ok_or("the light file has no textures")?;
    let ramp = psmt8_row_from_upload(ramp_entry, 4, 0, 256).map_err(error)?;
    let stage =
        StageResource::parse(&read(STAGE_FILE + setup.stage.max(0) as u32)?).map_err(error)?;
    let mut assets = BattleAssets {
        base_textures: Vec::new(),
        shade_textures: Vec::new(),
        fighters: Vec::new(),
        stage,
    };
    for side in setup.sides {
        let file_id = CHARA_FILE + side.chara.max(0) as u32 * 10 + side.costume.max(0) as u32;
        let file = read(file_id)?;
        let mut fighter = load_fighter(&file, &ramp, &mut assets)
            .map_err(|e| format!("character file 0x{file_id:X}: {e}"))?;
        let motions = read(MOTION_FILE + side.chara.max(0) as u32 * 10)?;
        let packed = pack_section(&motions, IDLE_MOTION + 1).map_err(error)?;
        let mut idle =
            Motion::parse(&bt3_data::bpe::decode(packed).map_err(error)?).map_err(error)?;
        // BtlAnim_Play: BtlAnim_ApplyTableFlags and BtlAnim_HideModelNodes.
        idle.apply_table_flags(motion_flags(&common, IDLE_MOTION));
        idle.hide_model_nodes(side.chara);
        fighter.idle = idle;
        assets.fighters.push(fighter);
    }
    Ok(assets)
}

/// `BtlAnim_GetFlags`: the roster's motion flag table is the table at header word 3 of common file 2.
fn motion_flags(common: &[u8], motion: usize) -> u32 {
    let word = |o: usize| {
        common
            .get(o..o + 4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    word((word(12) as usize & !3) + motion * 4)
}

/// `BtlObjMdl_BindFiles` and `ObjDraw_UploadTextures`: a batch's TEX0_1 is its material texture at
/// `tbp 0x3480 + tbpOfs`, `cbp 0x3C00 + cbpOfs` once `MdlTex_RebaseChain` moved it, and its TEX0_2 reads the
/// CLUT that `TexFile_UploadRange(clutFirst, clutCount)` put at `0x3D00 + tbpOfs`.
fn load_fighter(
    file: &[u8],
    ramp: &[u8],
    assets: &mut BattleAssets,
) -> Result<FighterModel, String> {
    let error = |e: std::io::Error| e.to_string();
    let model = CharaModel::from_file(file).map_err(error)?;
    let textures = TextureResource::parse(pack_section(file, TEXTURE_SECTION).map_err(error)?)
        .map_err(error)?;
    let header = &model.header;
    let entry = |i: u8| {
        textures
            .entries
            .get(usize::from(i))
            .ok_or_else(|| format!("texture {i} is missing"))
    };
    let mut materials = HashMap::new();
    for i in header.tex_first..header.tex_first.saturating_add(header.tex_count) {
        let e = entry(i)?;
        let mut texture = decode_sprite_texture(e).map_err(error)?;
        // TEX0_1 is MODULATE with alpha blending off (ALPHA_1 0x64 takes the source colour as it is).
        for pixel in texture.pixels.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        materials.insert(
            (e.tbp_offset as u32, e.cbp_offset as u32),
            assets.base_textures.len(),
        );
        assets.base_textures.push(texture);
    }
    let mut shades = HashMap::new();
    for i in header.clut_first..header.clut_first.saturating_add(header.clut_count) {
        let e = entry(i)?;
        let clut = decode_clut256_upload(e).map_err(error)?;
        let pixels = ramp
            .iter()
            .flat_map(|&index| {
                let [r, g, b, _] = clut[usize::from(index)];
                [r, g, b, 0xFF]
            })
            .collect();
        shades.insert(e.tbp_offset as u32, assets.shade_textures.len());
        assets.shade_textures.push(RgbaTexture {
            width: ramp.len() as u32,
            height: 1,
            pixels,
        });
    }
    let batch_textures = model
        .parts
        .iter()
        .map(|part| {
            part.batches
                .iter()
                .map(|batch| {
                    let key = (
                        tex0_tbp(batch.tex0[0]) - model.min_tbp,
                        tex0_cbp(batch.tex0[0]) - model.min_cbp,
                    );
                    let base = *materials
                        .get(&key)
                        .ok_or("a batch names no material texture")?;
                    let shade = *shades
                        .get(&tex0_cbp(batch.tex0[1]))
                        .ok_or("a batch names no shading CLUT")?;
                    Ok([base, shade])
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(FighterModel {
        model,
        batch_textures,
        idle: Motion {
            length: 0,
            tracks: Vec::new(),
        },
    })
}

/// One vertex as the GS gets it from program 0: screen position (pixels of the 512 x 448 frame) and Z, the
/// model texture's (s, t, 1) * q and the ramp's u * q.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenVertex {
    pub position: [f32; 3],
    pub stq: [f32; 3],
    pub uq: f32,
}

/// Triangles drawn with one material texture and one shading row.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelBatch {
    pub textures: [usize; 2],
    /// Three vertices per triangle.
    pub vertices: Vec<ScreenVertex>,
}

/// A fighter object as far as the draw needs it.
#[derive(Clone, Copy, Debug)]
struct Fighter {
    pos: [f32; 3],
    yaw: f32,
    /// `anim.frame` and layer 0's per-node key cursors.
    frame: f32,
    cursors: [u16; NODE_MAX],
}

/// The battle as far as it is ported.
pub struct Battle {
    pub assets: BattleAssets,
    fighters: [Fighter; 2],
    light_dir: [f32; 3],
}

impl Battle {
    pub fn new(assets: BattleAssets) -> Self {
        // The stage's starting places (`StgDataA.starts`, one per player): each fighter faces its target.
        let fighters = std::array::from_fn(|side| {
            let place = assets.stage.starting_placements.get(side);
            let pos = place.map_or([0.0; 3], |p| p.position);
            let target = place.map_or([0.0, 0.0, 1.0], |p| p.target);
            Fighter {
                pos,
                yaw: (target[0] - pos[0]).atan2(target[2] - pos[2]),
                frame: 0.0,
                cursors: [0; NODE_MAX],
            }
        });
        // BtlObj_UpdateAll: BtlObjLight_SetDir(BtlStage_GetLightVecB), which normalises it.
        let light_dir = normalize(assets.stage.light_dir.unwrap_or(DEFAULT_LIGHT));
        Self {
            assets,
            fighters,
            light_dir,
        }
    }

    /// One battle frame: each fighter advances its idle loop (`BtlAnim_AdvanceLoop`), its pose is sampled and its
    /// node matrices built, then the object draw: side 0's view (`BtlCam_GetDefaultView` outside split screen),
    /// every enabled part of both fighters through program 0.
    pub fn run_frame(&mut self) -> Vec<ModelBatch> {
        let mut poses = Vec::with_capacity(2);
        for (side, fighter) in self.fighters.iter_mut().enumerate() {
            let model = &self.assets.fighters[side];
            fighter.frame += ANIM_STEP;
            if fighter.frame >= f32::from(model.idle.length) - 0.1 {
                fighter.frame = 0.0;
            }
            let object = object_matrix(fighter);
            poses.push(pose(
                &model.model,
                &model.idle,
                fighter.frame,
                &mut fighter.cursors,
                &object,
            ));
        }
        let view = self.camera_view(0, &poses[0]);
        let mut groups: HashMap<[usize; 2], Vec<ScreenVertex>> = HashMap::new();
        for (side, nodes) in poses.iter().enumerate() {
            let model = &self.assets.fighters[side];
            let parts = model
                .model
                .parts
                .iter()
                .zip(&model.batch_textures)
                .zip(nodes);
            for ((part, textures), (mtx_a, mtx_b)) in parts {
                if part.enabled && node_shown(model.model.header.flags, part.node) {
                    draw_part(
                        part,
                        textures,
                        mtx_a,
                        mtx_b,
                        self.light_dir,
                        &view,
                        &mut groups,
                    );
                }
            }
        }
        let mut batches: Vec<ModelBatch> = groups
            .into_iter()
            .map(|(textures, vertices)| ModelBatch { textures, vertices })
            .collect();
        batches.sort_by_key(|batch| batch.textures);
        batches
    }

    /// `ChrCam_Update` with the lock-on camera (`ChrCam_CalcLockOn`) of a fighter standing still and facing its
    /// opponent: `cam->yaw` has reached the opponent's direction and `cam->side` is still 0. No shake, no bob.
    fn camera_view(&self, side: usize, own_pose: &[(Mtx, Mtx)]) -> View {
        let own = self.fighters[side];
        let opp = self.fighters[side ^ 1];
        let own_model = &self.assets.fighters[side].model;
        let height = own_model.header.size[0];
        let opp_height = self.assets.fighters[side ^ 1].model.header.size[0];
        let stage = &self.assets.stage;
        let cam_yaw = (opp.pos[0] - own.pos[0]).atan2(opp.pos[2] - own.pos[2]);
        let (mut eye, rot, mut target) = lock_on_camera(&LockOn {
            pos: own.pos,
            height,
            side_slope: own_model.header.size[3],
            opp_pos: opp.pos,
            opp_height,
            stage_bottom: stage.bounds.bottom,
            yaw: cam_yaw,
            side: 0.0,
        });
        // The eye and target stay inside the stage's radius.
        for point in [&mut eye, &mut target] {
            let len = point[0].hypot(point[2]);
            if stage.bounds.radius < len {
                let k = stage.bounds.radius / len;
                point[0] *= k;
                point[2] *= k;
            }
        }
        // ChrCam_KeepBehindHead: node 0x30's world position.
        if let Some(index) = own_model.parts.iter().position(|p| p.node == HEAD_NODE) {
            let head = own_pose[index].0[3];
            keep_behind_head([head[0], head[1], head[2]], height, &mut eye, rot);
        }
        View::new(eye, rot)
    }
}

/// The fighter object's placement (`BtlObjXf_Update`): the model faces -Z, so the yaw is turned by pi, then
/// `Vu0Cur_RotateZXY` and the position.
fn object_matrix(fighter: &Fighter) -> Mtx {
    let (s, c) = wrap_angle(fighter.yaw + PI).sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [fighter.pos[0], fighter.pos[1], fighter.pos[2], 1.0],
    ]
}

/// `BtlObj_IsNodeShown` with all of the object's part flags clear (`visible && partN` never holds): the
/// alternative parts that need a flag stay hidden.
fn node_shown(model_flags: u32, node: u16) -> bool {
    const PARTS_A: u32 = 0x1_0000;
    const PARTS_B: u32 = 0x2_0000;
    const PARTS_C: u32 = 0x4_0000;
    if model_flags & PARTS_A != 0 {
        !matches!(node, 0x42 | 0x43)
    } else if model_flags & PARTS_B != 0 {
        node != 0x40
    } else if model_flags & PARTS_C != 0 {
        !matches!(node, 0x40 | 0x41)
    } else {
        !matches!(node, 0x40 | 0x41 | 0x6F)
    }
}

/// `BtlObjAnim_SamplePose` (layer 0 only, no blend) and `BtlObjPose_CalcMatrices`: per mesh record, the node's
/// world matrix and its parent's (what `ObjDraw_DrawParts` hands program 0 as bones A and B). The records are
/// the skeleton in parent-first order; each pushes the current matrix and pops `pop` after itself.
fn pose(
    model: &CharaModel,
    motion: &Motion,
    frame: f32,
    cursors: &mut [u16; NODE_MAX],
    object: &Mtx,
) -> Vec<(Mtx, Mtx)> {
    let mut stack = Vec::new();
    let mut current = *object;
    let mut nodes = Vec::with_capacity(model.parts.len());
    for part in &model.parts {
        let id = usize::from(part.node);
        let rest = [part.rest[0], part.rest[1], part.rest[2]];
        let track = motion.tracks.get(id).and_then(Option::as_ref);
        let (pos, rot) = match track {
            Some(track) if id < NODE_MAX => {
                let (pos, rot) = track.sample(&mut cursors[id], frame);
                (pos.unwrap_or(rest), rot)
            }
            _ => (rest, [0.0, 0.0, 0.0, 1.0]),
        };
        let mut local = quat_to_mtx(rot);
        local[3] = [pos[0], pos[1], pos[2], 1.0];
        stack.push(current);
        let parent = current;
        current = mul(&local, &parent);
        nodes.push((current, parent));
        for _ in 0..part.pop {
            current = stack.pop().unwrap_or(*object);
        }
    }
    nodes
}

/// `Quat_ToMtx`.
fn quat_to_mtx([x, y, z, w]: [f32; 4]) -> Mtx {
    let (xx, yy, zz) = (x * x, y * y, z * z);
    [
        [
            1.0 - 2.0 * yy - 2.0 * zz,
            2.0 * x * y + 2.0 * w * z,
            2.0 * x * z - 2.0 * w * y,
            0.0,
        ],
        [
            2.0 * x * y - 2.0 * w * z,
            1.0 - 2.0 * xx - 2.0 * zz,
            2.0 * y * z + 2.0 * w * x,
            0.0,
        ],
        [
            2.0 * x * z + 2.0 * w * y,
            2.0 * y * z - 2.0 * w * x,
            1.0 - 2.0 * xx - 2.0 * yy,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// What `ChrCam_CalcLockOn` reads.
struct LockOn {
    pos: [f32; 3],
    height: f32,
    side_slope: f32,
    opp_pos: [f32; 3],
    opp_height: f32,
    stage_bottom: f32,
    yaw: f32,
    side: f32,
}

/// `ChrCam_GetOffset` for distance preset 0: the offset behind the fighter and the pitch.
fn camera_offset(height: f32) -> ([f32; 3], f32) {
    let dist = (height * 0.5 + 1.0).max(10.0);
    ([0.0, -dist * 1.2, -dist * 2.4], height * -0.001 + -0.05)
}

/// `ChrCam_CalcLockOn`: returns the eye, the rotation (pitch, yaw, 0) and the target.
fn lock_on_camera(cam: &LockOn) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let scale = cam.height;
    let mut pos = cam.pos;
    let floor_y = cam.stage_bottom + scale * 0.5;
    if floor_y < pos[1] {
        pos[1] = floor_y;
    }
    let mut to_opp = sub(cam.opp_pos, pos);
    to_opp[1] += cam.height * 0.5 - cam.opp_height * 0.5;
    let len = length(to_opp);
    let dir = if len < 0.01 {
        [0.0, 0.0, 1.0]
    } else {
        scale3(to_opp, 1.0 / len)
    };
    let mut target = [0.0, -(scale * 0.7), 0.0];
    let (ofs, mut base) = camera_offset(scale);
    let mut ofs = sub(ofs, target);
    let mut u = if len < 0.0 {
        0.0
    } else if len < 100.0 {
        len / 100.0
    } else {
        1.0
    };
    u = u * 2.0 - 1.0;
    u = -u * u.abs() + u * 2.0;
    u = u * 0.5 + 0.5;
    u = u * 0.2 + 0.8;
    ofs = scale3(ofs, u);
    target = scale3(target, 1.0 - (1.0 - u) * 0.5);
    let max_pitch = PI * 0.35;
    let mut a = (-dir[1].clamp(-1.0, 1.0).asin() / max_pitch).clamp(-1.0, 1.0);
    let mut b = a.max(0.0);
    b = b * 2.0 - 1.0;
    a = -a * a.abs() + a * 2.0;
    b = -b * b.abs() + b * 2.0;
    let pitch = max_pitch * a * 0.4;
    b = b * 0.5 + 0.5;
    ofs = rotate_x(ofs, pitch);
    ofs = scale3(ofs, 1.0 - b * 0.3);
    base += pitch - base * b;
    let limit = cam.side_slope.atan().max(0.785_398_2);
    let horiz = to_opp[0].hypot(to_opp[2]);
    let ofs_len = ofs[0].hypot(ofs[2]);
    let fov = (512.0f32 * 0.5 / 433.0).atan() * 2.0;
    let half = fov * 0.7;
    let tn = half.tan();
    let spread = if 0.01 < horiz {
        (tn * (ofs_len / horiz)).atan()
    } else if 0.0 < tn * ofs_len * horiz {
        FRAC_PI_2
    } else {
        -FRAC_PI_2
    };
    let side = cam.side / limit;
    let mut turn = half + spread * (limit - fov) / limit;
    ofs = rotate_y(ofs, turn * side);
    turn += spread * (limit / FRAC_PI_2);
    turn *= 0.5;
    turn *= side;
    let yaw = wrap_angle(cam.yaw - turn);
    target = add(target, pos);
    let (s, c) = cam.yaw.sin_cos();
    let out = [-c * ofs[0] + s * ofs[2], ofs[1], s * ofs[0] + c * ofs[2]];
    (add(target, out), [base, yaw, 0.0], target)
}

/// `ChrCam_KeepBehindHead`: pushes the eye back onto a vertical plane behind the head.
fn keep_behind_head(node: [f32; 3], height: f32, eye: &mut [f32; 3], rot: [f32; 3]) {
    let margin = height * 0.15;
    let dir = [rot[1].sin(), 0.0, rot[1].cos()];
    let w = -dot(dir, node) + margin * (rot[0].cos() + 1.0) + 10.0;
    let d = dot(dir, *eye) + w;
    if 0.0 < d {
        *eye = sub(*eye, scale3(dir, d));
    }
}

/// `View_SetTransform` + `View_UpdateMatrices` with `View_InitLayout(VIEW_LAYOUT_FULL)`'s projection.
struct View {
    world2screen: Mtx,
    world2clip: Mtx,
}

impl View {
    fn new(pos: [f32; 3], rot: [f32; 3]) -> Self {
        let world2view = view_matrix(pos, rot);
        let (view2screen, view2clip) = projection();
        Self {
            world2screen: mul(&world2view, &view2screen),
            world2clip: mul(&world2view, &view2clip),
        }
    }
}

/// `Vu0View_SetRotTrans(rot, -pos)`: rotation about X, then Z, then Y (inverse angles), translation first.
fn view_matrix(pos: [f32; 3], rot: [f32; 3]) -> Mtx {
    let mut m = IDENTITY;
    let (s, c) = rot[0].sin_cos();
    m[1] = [0.0, c, -s, 0.0];
    m[2] = [0.0, s, c, 0.0];
    let (s, c) = rot[2].sin_cos();
    let (m0, m1) = (m[0], m[1]);
    m[0] = lerp_rows(m0, c, m1, -s);
    m[1] = lerp_rows(m0, s, m1, c);
    let (s, c) = rot[1].sin_cos();
    let (m0, m2) = (m[0], m[2]);
    m[0] = lerp_rows(m0, c, m2, s);
    m[2] = lerp_rows(m0, -s, m2, c);
    let t = [-pos[0], -pos[1], -pos[2]];
    for k in 0..3 {
        m[3][k] = t[0] * m[0][k] + t[1] * m[1][k] + t[2] * m[2][k];
    }
    m[3][3] = 1.0;
    m
}

fn lerp_rows(a: [f32; 4], ka: f32, b: [f32; 4], kb: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] * ka + b[i] * kb)
}

/// `View_BuildProjection` for the full-screen layout: screen size 2047, distance 433, aspect (1 / 1.1666667, 1),
/// centre 2048, Z from 10877000 (near 0.3) to 1 (far 65536), scale 1.
fn projection() -> (Mtx, Mtx) {
    let (size, dist, scale) = (2047.0f32, 433.0f32, 1.0f32);
    let (ax, ay) = (1.0 / 1.166_666_7f32, 1.0f32);
    let (center_x, center_y) = (2048.0f32, 2048.0f32);
    let (z_min, z_max, near, far) = (1.0f32, 10_877_000.0f32, 0.3f32, 65536.0f32);
    let z_ofs = (-z_max * near + z_min * far) / (far - near);
    let hw = near * size / dist;
    let hh = near * size / dist;
    let z_scale = far * near * (z_max - z_min) / (far - near);
    let ox = (center_x - 2048.0) / (size * ax);
    let oy = (center_y - 2048.0) / (size * ay);
    let mut persp = IDENTITY;
    persp[0][0] = dist * scale;
    persp[1][1] = dist * scale;
    persp[2][2] = 0.0;
    persp[3][2] = 1.0;
    persp[2][3] = 1.0;
    persp[3][3] = 0.0;
    let mut screen = IDENTITY;
    screen[0][0] = ax;
    screen[1][1] = ay;
    screen[2][2] = z_scale;
    screen[3] = [center_x, center_y, z_ofs, 1.0];
    let mut clip = IDENTITY;
    clip[0][0] = 2.0 * near / (2.0 * hw) * scale;
    clip[1][1] = 2.0 * near / (2.0 * hh) * scale;
    clip[2][2] = (far + near) / (far - near);
    clip[2][3] = 1.0;
    clip[3][2] = far * near * -2.0 / (far - near);
    clip[3][3] = 0.0;
    let mut shift = IDENTITY;
    shift[3][0] = ox;
    shift[3][1] = oy;
    (mul(&persp, &screen), mul(&clip, &shift))
}

/// `ObjDraw_DrawParts` mode 0 for one part and VU1 program 0 on its batches: bone A is the part's node, bone B
/// its parent; `offset_a` / `offset_b` are their rest positions in model space.
fn draw_part(
    part: &ModelPart,
    textures: &[[usize; 2]],
    mtx_a: &Mtx,
    mtx_b: &Mtx,
    light_dir: [f32; 3],
    view: &View,
    groups: &mut HashMap<[usize; 2], Vec<ScreenVertex>>,
) {
    // Mtx_MakeNormalLight(light, lightDir, 0, 0): x of light * n is -L . n; Vec3_Normalize clears w.
    let neg_light = scale3(normalize(light_dir), -1.0);
    for (batch, &tex) in part.batches.iter().zip(textures) {
        let mut flags = [false; 2];
        let mut previous: [Option<ScreenVertex>; 2] = [None, None];
        let out = groups.entry(tex).or_default();
        for vertex in &batch.vertices {
            let [x, y, z, w] = vertex.position;
            let pa = apply(
                &mtx_a,
                [
                    x - part.offset_a[0],
                    y - part.offset_a[1],
                    z - part.offset_a[2],
                    1.0,
                ],
            );
            let pb = apply(
                &mtx_b,
                [
                    x - part.offset_b[0],
                    y - part.offset_b[1],
                    z - part.offset_b[2],
                    1.0,
                ],
            );
            let p: [f32; 4] = std::array::from_fn(|i| pb[i] + (pa[i] - pb[i]) * w);
            let [nx, ny, nz] = vertex.normal;
            let la = dot(neg_light, rotate(&mtx_a, [nx, ny, nz]));
            let lb = dot(neg_light, rotate(&mtx_b, [nx, ny, nz]));
            let u = 0.5 + 0.5 * (lb + (la - lb) * w);
            let s = apply(&view.world2screen, [p[0], p[1], p[2], 1.0]);
            let c = apply(&view.world2clip, [p[0], p[1], p[2], 1.0]);
            let q = 1.0 / s[3];
            // ftoi4 for x / y; the GS takes Z from bits 4..27 of the packed word, so whole units.
            let sx = (s[0] * q * 16.0).trunc() / 16.0;
            let sy = (s[1] * q * 16.0).trunc() / 16.0;
            let sz = (s[2] * q).trunc();
            let cw = c[3].abs();
            let clipped = c[0].abs() > cw || c[1].abs() > cw || c[2].abs() > cw;
            let screen = ScreenVertex {
                position: [sx - OFFSET_X, sy - OFFSET_Y, sz],
                stq: [vertex.st[0] * q, vertex.st[1] * q, q],
                uq: u * q,
            };
            // ADC: a vertex closes no triangle when it or one of the two before it is outside the clip volume.
            if let [Some(a), Some(b)] = previous
                && !(clipped || flags[0] || flags[1])
            {
                out.extend([a, b, screen]);
            }
            flags = [flags[1], clipped];
            previous = [previous[1], Some(screen)];
        }
    }
}

fn apply(m: &Mtx, v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|k| v[0] * m[0][k] + v[1] * m[1][k] + v[2] * m[2][k] + v[3] * m[3][k])
}

fn rotate(m: &Mtx, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| v[0] * m[0][k] + v[1] * m[1][k] + v[2] * m[2][k])
}

/// `a` then `b` for row vectors.
fn mul(a: &Mtx, b: &Mtx) -> Mtx {
    std::array::from_fn(|i| apply(b, a[i]))
}

/// `Vec4_RotateEuler` with only an X angle (`Vu0Cur_RotateX`).
fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

/// `Vec4_RotateEuler` with only a Y angle (`Vu0Cur_RotateY`).
fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] + s * v[2], v[1], -s * v[0] + c * v[2]]
}

/// `BtlUtil_WrapAngle`: into [-pi, pi).
fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    scale3(a, 1.0 / length(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_puts_the_view_axis_at_the_screen_centre_and_near_at_z_max() {
        let (screen, _) = projection();
        let s = apply(&screen, [0.0, 0.0, 0.3, 1.0]);
        assert_eq!(s[0] / s[3], 2048.0);
        assert_eq!(s[1] / s[3], 2048.0);
        assert!((s[2] / s[3] - 10_877_000.0).abs() < 4.0);
        let far = apply(&screen, [0.0, 0.0, 65536.0, 1.0]);
        assert!((far[2] / far[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn view_matrix_looks_along_the_yaw() {
        let m = view_matrix([1.0, 2.0, 3.0], [0.0, FRAC_PI_2, 0.0]);
        // Yaw pi/2 looks along +x: a point ahead of the camera ends up on the view's +z axis.
        let v = apply(&m, [11.0, 2.0, 3.0, 1.0]);
        assert!(v[0].abs() < 1e-5 && v[1].abs() < 1e-5 && (v[2] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn lock_on_camera_sits_behind_and_above_the_fighter() {
        let cam = LockOn {
            pos: [0.0, 0.0, 0.0],
            height: 20.0,
            side_slope: 3.0,
            opp_pos: [0.0, 0.0, 100.0],
            opp_height: 20.0,
            stage_bottom: 1000.0,
            yaw: 0.0,
            side: 0.0,
        };
        let (eye, rot, target) = lock_on_camera(&cam);
        assert!(eye[2] < 0.0 && eye[1] < 0.0 && eye[0].abs() < 1e-5);
        assert_eq!(target, [0.0, -14.0, 0.0]);
        assert_eq!(rot[1], 0.0);
        assert!(rot[0] < 0.0);
    }

    #[test]
    fn real_fighters_load_and_project_onto_the_screen() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = bt3_data::Iso9660::open(&path) else {
            return;
        };
        let mut read = |id: u32| -> Result<Vec<u8>, String> {
            let location = bt3_data::afs::partition_entry_for_id(id);
            let archive = bt3_data::afs::AfsArchive::open(
                &mut image,
                format!("DATA/PZS3US{}.AFS", location.partition),
            )
            .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        };
        let mut setup = BattleSetup::default();
        setup.sides[0].chara = 0;
        setup.sides[1].chara = 1;
        setup.stage = 0;
        let assets = load(&mut read, &setup).unwrap();
        let mut battle = Battle::new(assets);
        assert_eq!(battle.assets.fighters[0].idle.length, 67);
        let batches = battle.run_frame();
        let vertices: Vec<_> = batches.iter().flat_map(|b| &b.vertices).collect();
        assert!(vertices.len() > 3000);
        // The two fighters stand in front of the camera: most of what is drawn is on screen.
        let on_screen = vertices
            .iter()
            .filter(|v| {
                (0.0..512.0).contains(&v.position[0]) && (0.0..448.0).contains(&v.position[1])
            })
            .count();
        assert!(on_screen * 2 > vertices.len());
        // The idle stance: ankles (node 11) on the ground, hips (node 2) lowered, the head (node 0x30) above them
        // and leaning forward (-Z is the model's front); y points down.
        let fighter = &battle.assets.fighters[0];
        let mut cursors = [0; NODE_MAX];
        let nodes = pose(&fighter.model, &fighter.idle, 10.0, &mut cursors, &IDENTITY);
        let at = |id: u16| {
            let i = fighter
                .model
                .parts
                .iter()
                .position(|p| p.node == id)
                .unwrap();
            nodes[i].0[3]
        };
        assert!((at(11)[1] + 1.2).abs() < 0.5);
        assert!(at(2)[1] > -8.0 && at(2)[1] < -5.0);
        assert!(at(HEAD_NODE)[1] < at(2)[1] - 3.0 && at(HEAD_NODE)[2] < -2.0);
    }
}
