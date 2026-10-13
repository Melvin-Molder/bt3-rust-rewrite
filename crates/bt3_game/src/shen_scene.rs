//! The 3D backdrop of the wish screen (`ShenScene`, src/ui/shen_scene.c): like the model viewer it runs the
//! battle's object, stage and screen-effect modules without a battle. A backdrop (stage file 0x194 + dragon), the
//! seven balls and the dragon, with the two camera animations of the dragon's model file (`charCamAnims`, sections
//! 0x25 / 0x26). The summoning is a fixed sequence (`ShenScene_StepSeq`): the balls pulsing under the first camera,
//! the radial blur from camera time 354, then the dragon in front of a fixed camera that shakes, with a shock wave
//! and a cross-fade, until the screen asks to leave; a farewell camera, a fade out.
//!
//! Only layer 3 of the scene runs (`BtlScene_Init(8)`), and no stage settings are loaded, so the stage's tints
//! (`StgFx_DrawPreNoCheck`) draw nothing, as in the model viewer. The objects' glow (`BtlObj_SetAlphaAdd`, the
//! frame-alpha pass `ObjGlow` of `BtlObj_BeginDraw`) is kept but not drawn: the battle renderer does not have that
//! pass.

use std::collections::HashMap;

use bt3_data::cam_anim::CamAnim;
use bt3_data::gs::RgbaTexture;
use bt3_data::stage_model::StageModel;

use crate::battle::{IDENTITY, ModelBatch, ModelSink, View};
use crate::battle_seq::{DemoCam, Ramp, ScrXfade};
use crate::btl_obj::Obj;
use crate::eft::DrawCtx;
use crate::eft_obj::EffectModel;
use crate::fade::Fade;
use crate::post::PostOp;
use crate::rng::{LibcRandom, Vu0Random};
use crate::stage_draw::StageBatch;
use crate::title::TitleSound;

/// `SHENSCENE_FILE_STAGE`: the backdrop of dragon `d` is stage file `0x194 + d`.
const FILE_STAGE: u32 = 0x194;
/// `ShenScene_StepLoad`'s model files: the dragon's and the balls', by dragon.
const FILES: [[u32; 2]; 3] = [[0xD3C, 0xD46], [0xD3D, 0xD46], [0xD45, 0xD47]];
/// The model file's camera animations at object + 0x908 (`BtlObj_BindTables`: `mdl.charCamAnims[i]`, the model
/// block being at object + 0x18, is section `0x25 + i`).
const CAM_SECTION: usize = 0x25;
const DRAGON: usize = 0;
const BALLS: usize = 1;

// SHENSCENE_STATE_*.
pub(crate) const STATE_INTRO: i32 = 0;
pub(crate) const STATE_READY: i32 = 1;
pub(crate) const STATE_ENDED: i32 = 2;
pub(crate) const STATE_LEAVE: i32 = 3;

/// `BOBJ_PLAY_*`.
const PLAY_NONE: u8 = 0;
const PLAY_ONCE: u8 = 1;
const PLAY_LOOP: u8 = 2;
/// Object flags: drawn (2), and the one the objects are made with that the sequence clears.
const OBJ_VISIBLE: u32 = 2;
const OBJ_HIDE_EVENT: u32 = 0x0200_0000;

/// The fixed camera of each dragon (`ShenScene_StepSeq` step 2): position, rotation in degrees.
const CAM_POS: [[f32; 3]; 3] = [
    [0.86, -90.9870, -19.1110],
    [-0.891, -169.7360, -33.2710],
    [-0.399, -2.547, -10.358],
];
const CAM_ROT: [[f32; 3]; 3] = [
    [10.72, 0.0, 0.0],
    [16.778, -0.823, 0.0],
    [5.142, 3.569, 0.0],
];

/// The textures the scene's models bind (`ModelSink`).
#[derive(Default)]
struct Textures {
    base: Vec<RgbaTexture>,
    shade: Vec<RgbaTexture>,
    ids: Vec<RgbaTexture>,
}

impl ModelSink for Textures {
    fn base_textures(&mut self) -> &mut Vec<RgbaTexture> {
        &mut self.base
    }

    fn shade_textures(&mut self) -> &mut Vec<RgbaTexture> {
        &mut self.shade
    }

    fn id_textures(&mut self) -> &mut Vec<RgbaTexture> {
        &mut self.ids
    }

    fn face_tables(&self) -> crate::btl_face::FaceTables {
        crate::btl_face::FaceTables::default()
    }
}

/// `ShenActor`: an object, its model, how its animation plays, its glow ramp and value.
struct Actor {
    model: EffectModel,
    obj: Obj,
    mode: u8,
    ramp: Ramp,
    alpha_add: i32,
}

impl Actor {
    /// `BtlObjAnim_PlayModel(obj, anim, mode)`.
    fn play(&mut self, anim: usize, mode: u8) {
        self.obj.play(&self.model.data(), 0, anim, true);
        self.mode = if self.obj.layers[0].motion.is_some() {
            mode
        } else {
            PLAY_NONE
        };
    }

    /// `BtlObj_UpdateAll` for the object: its animation stepped (`BtlObjAnim_Step`), the pose and the nodes.
    fn update(&mut self) {
        let data = self.model.data();
        let length = self.obj.layers[0].length;
        match self.mode {
            PLAY_ONCE => {
                self.obj.prev_frame = self.obj.frame;
                self.obj.frame += self.obj.step;
                if length <= self.obj.frame {
                    self.mode = PLAY_NONE;
                    self.obj.frame = length;
                    self.obj.prev_frame = length;
                }
            }
            PLAY_LOOP => {
                self.obj.prev_frame = self.obj.frame;
                self.obj.frame += self.obj.step;
                if length <= self.obj.frame {
                    self.obj.frame = 0.0;
                    self.obj.prev_frame = 0.0;
                }
            }
            _ => {}
        }
        self.obj.sample_pose(&data);
        self.obj.calc_matrices(&data);
    }

    /// `Ramp_Step`, turning a pulse around at its ends, then `BtlObj_SetAlphaAdd` with its value.
    fn pulse(&mut self, seconds: f32, high: f32) {
        if self.ramp.step() {
            if self.ramp.value == 0.0 {
                self.ramp.start(seconds, 0.0, high);
            } else {
                self.ramp.start(seconds, high, 0.0);
            }
        }
        self.alpha_add = self.ramp.value as u32 as u8 as i32;
    }
}

/// `ShenSeq`.
#[derive(Default)]
struct Seq {
    /// `fn` is set: the sequence runs.
    running: bool,
    step: i32,
    se_step: i32,
    cams: [Option<CamAnim>; 2],
    cam_idx: usize,
    state: i32,
}

/// What a frame of the scene draws, in `ShenScene_Update`'s order.
#[derive(Default)]
pub(crate) struct SceneFrame {
    pub stage: Vec<StageBatch>,
    pub models: Vec<ModelBatch>,
    /// `StgFx_DrawPost`: the radial blur and the shock waves.
    pub post: Vec<PostOp>,
    /// `BtlObj_BeginDraw`'s passes, while an object is drawn.
    pub obj_alpha: Option<crate::battle::ObjAlpha>,
    /// `StgFx_DrawOverlay`'s cross-fade: keep this frame, and the alpha of the kept one over it.
    pub capture: bool,
    pub xfade_alpha: i32,
    pub sounds: Vec<TitleSound>,
}

/// `ShenScene`.
pub(crate) struct ShenScene {
    dragon: usize,
    stage: StageModel,
    stage_id: i32,
    pub stage_textures: Vec<RgbaTexture>,
    textures: Textures,
    light_dir: [f32; 3],
    /// `BtlStage_GetAmbient`: the stage's ambient colour.
    ambient: [u8; 3],
    actors: [Actor; 2],
    /// The objects' rows of the alpha table.
    alpha_rows: crate::btl_obj::AlphaRows,
    seq: Seq,
    demo: DemoCam,
    view: View,
    blurs: crate::stg_blur::Blurs,
    warps: crate::scr_warp::ScrWarps,
    xfade: ScrXfade,
    /// Fade slot 0 (`Fade_Start(0, …)`), drawn by `Gfx_EndFrame` over everything.
    pub fade: Fade,
    time: f32,
    frame: i32,
}

impl ShenScene {
    /// `ShenScene_Init(dragon)`: the backdrop and the two models read (`ShenScene_StepLoad`, blocking), the
    /// objects made (the dragon of type 1, the balls of type 3), the sequence started and a fade in.
    pub(crate) fn new(
        read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
        dragon: usize,
    ) -> Result<Self, String> {
        let dragon = dragon.min(2);
        let common = read(crate::battle::COMMON_LIGHT_FILE)?;
        let ramp = crate::battle::light_ramp(&common)?;
        let stage_id = (FILE_STAGE + dragon as u32 - crate::battle::STAGE_FILE) as i32;
        let loaded = crate::battle::load_stage(read, stage_id, false)?;
        let light_dir = crate::battle::normalize(
            loaded
                .stage
                .light_dir
                .unwrap_or(crate::battle::DEFAULT_LIGHT),
        );
        let mut textures = Textures::default();
        let mut cams = [None, None];
        let mut actor = |i: usize, textures: &mut Textures| -> Result<Actor, String> {
            let file = read(FILES[dragon][i])?;
            let model = crate::battle::load_effect_model(&file, &ramp, textures)?;
            if i == DRAGON {
                // BtlObj_BindTables: the dragon's camera animations (obj + 0x908, `charCamAnims`).
                for (k, cam) in cams.iter_mut().enumerate() {
                    *cam = bt3_data::pack_entry(&file, CAM_SECTION + k)
                        .and_then(|b| CamAnim::parse(b).ok());
                }
            }
            let mut obj = Obj::new(&model.data(), 0.0);
            // BtlObj_InitFlags: type 1 0x2000006, type 3 0x22000016; 8 for a fading model; the colour flag.
            let color = obj.flags & 0x3_0000;
            obj.flags = color | if i == DRAGON { 0x200_0006 } else { 0x2200_0016 };
            if model.model.header.flags & 1 != 0 {
                obj.flags |= 8;
            }
            // BtlObj_SetDefaultAnimStep(2.0).
            obj.step = 2.0;
            Ok(Actor {
                model,
                obj,
                mode: PLAY_NONE,
                ramp: Ramp::default(),
                alpha_add: 0,
            })
        };
        let actors = [actor(DRAGON, &mut textures)?, actor(BALLS, &mut textures)?];
        // ShenScene_StartSeq: the first camera is the next to play.
        let seq = Seq {
            running: true,
            cams,
            ..Seq::default()
        };
        Ok(Self {
            dragon,
            stage: loaded.model,
            stage_id,
            stage_textures: loaded.textures,
            textures,
            light_dir,
            ambient: loaded.stage.ambient,
            actors,
            alpha_rows: crate::btl_obj::AlphaRows::default(),
            seq,
            demo: DemoCam::default(),
            view: View::new([0.0; 3], [0.0; 3]),
            blurs: crate::stg_blur::Blurs::default(),
            warps: crate::scr_warp::ScrWarps::default(),
            xfade: ScrXfade::default(),
            fade: Fade::start_in(1.0),
            time: 0.0,
            frame: 0,
        })
    }

    /// The models' textures, as the renderer takes them.
    pub(crate) fn model_textures(&self) -> crate::battle::ModelTextures {
        (
            self.textures.base.clone(),
            self.textures.shade.clone(),
            self.textures.ids.clone(),
        )
    }

    /// `ShenScene_GetState`.
    pub(crate) fn state(&self) -> i32 {
        self.seq.state
    }

    /// `ShenScene_SetState`: LEAVE is only taken while the scene is READY.
    pub(crate) fn set_state(&mut self, state: i32) {
        if state == STATE_LEAVE && self.state() != STATE_READY {
            return;
        }
        self.seq.state = state;
    }

    /// `ShenScene_NextCam`: the next camera animation from its start; false when none is left.
    fn next_cam(&mut self) -> bool {
        let Some(cam) = self.seq.cams.get(self.seq.cam_idx).and_then(Clone::clone) else {
            return false;
        };
        self.demo.start_auto(cam);
        self.seq.cam_idx += 1;
        true
    }

    /// `ShenScene_IsCamEnd`.
    fn is_cam_end(&self) -> bool {
        self.demo.time() >= self.demo.length()
    }

    /// `ShenScene_PlaySe`: a sound of bank mask 2, queued twice.
    fn play_se(id: u32, volume: i32, sounds: &mut Vec<TitleSound>) {
        for _ in 0..2 {
            sounds.push(TitleSound::SeEx {
                mask: 2,
                id,
                volume,
                pan: 0,
                pitch: 0,
                handle: 0,
            });
        }
    }

    /// The radial blur's four layers: grey with these alphas.
    fn set_blur(&mut self, alpha3: u8, alpha0: u8, alpha1: u8, alpha2: u8) {
        for (which, a) in [(3, alpha3), (0, alpha0), (1, alpha1), (2, alpha2)] {
            self.blurs.set_color(0, which, [0x80, 0x80, 0x80, a]);
        }
    }

    /// `ShenScene_StepSeq`: one step of the summoning; true when it is over.
    fn step_seq(&mut self, vu0: &mut Vu0Random, sounds: &mut Vec<TitleSound>) -> bool {
        let mut step = self.seq.step;
        loop {
            match step {
                0 => {
                    self.next_cam();
                    for (i, a) in self.actors.iter_mut().enumerate() {
                        a.obj.flags &= !OBJ_HIDE_EVENT;
                        a.obj.mtx = IDENTITY;
                        a.obj.pos = [0.0; 3];
                        a.play(0, PLAY_LOOP);
                        if i == DRAGON {
                            a.alpha_add = 0x40;
                        }
                    }
                    let balls = &mut self.actors[BALLS];
                    balls.ramp.start(0.5, 0.0, 127.0);
                    balls.alpha_add = balls.ramp.value as u32 as u8 as i32;
                    self.seq.step += 1;
                    break;
                }
                1 => {
                    self.actors[BALLS].pulse(0.5, 127.0);
                    if self.demo.time() >= 354.0 {
                        // Step 3's blur at strength 1.
                        self.set_blur(64, 0, 0, 128);
                        self.blurs.set_center(0, [0.0, 0.0, 1.0], false);
                    }
                    if !self.is_cam_end() {
                        break;
                    }
                    self.seq.step += 1;
                    step = 2;
                }
                2 => {
                    let d = &mut self.actors[DRAGON];
                    d.play(1, PLAY_LOOP);
                    d.ramp.start(2.0, 64.0, 0.0);
                    self.actors[BALLS].obj.flags &= !OBJ_VISIBLE;
                    let rot = CAM_ROT[self.dragon].map(|a| a * std::f32::consts::PI / 180.0);
                    let pos = CAM_POS[self.dragon];
                    self.demo.set_fixed(pos, rot);
                    self.demo.add_shake(1.0);
                    // The shock wave at the camera's height, on the axis.
                    self.warps
                        .spawn(0, [0.0, pos[1], 0.0], 3.8, 10.0, 50.0, 10.0, 0.5, vu0);
                    self.xfade.request_capture();
                    self.xfade.start(0.5);
                    self.set_state(STATE_READY);
                    self.seq.step += 1;
                    step = 3;
                }
                3 => {
                    self.actors[DRAGON].pulse(2.0, 64.0);
                    let shake = self.demo.shake_time();
                    let a = |k: f32| (shake * k) as u32 as u8;
                    self.set_blur(a(96.0), a(0.0), a(52.0), a(128.0));
                    self.blurs.set_center(0, [0.0, -0.3, 0.55], false);
                    if self.state() != STATE_LEAVE {
                        break;
                    }
                    self.set_state(STATE_INTRO);
                    self.xfade.request_capture();
                    self.xfade.start(1.0);
                    self.seq.step += 1;
                    break;
                }
                4 => {
                    self.demo.clear_fixed();
                    self.next_cam();
                    self.actors[DRAGON].obj.flags &= !OBJ_VISIBLE;
                    let balls = &mut self.actors[BALLS];
                    balls.obj.flags |= OBJ_VISIBLE;
                    balls.play(1, PLAY_ONCE);
                    balls.alpha_add = 0x7F;
                    self.seq.step += 1;
                    step = 5;
                }
                5 => {
                    if self.is_cam_end() {
                        self.fade = Fade::start_out(1.0);
                        self.seq.step += 1;
                    }
                    break;
                }
                6 => {
                    if self.fade.is_done() {
                        return true;
                    }
                    break;
                }
                _ => return true,
            }
        }
        // The sounds that follow the steps.
        match self.seq.se_step {
            0 => {
                if self.seq.step < 2 {
                    if self.demo.time() < 354.0 {
                        let level = 1.0 - self.demo.time() / 420.0;
                        if self.frame % 27 == 0 {
                            Self::play_se(0x3D, (level * 127.0) as i32, sounds);
                        }
                    } else {
                        Self::play_se(0x3E, 0x7F, sounds);
                        self.seq.se_step += 1;
                    }
                }
            }
            1 => {
                if self.seq.step == 4 {
                    Self::play_se(0x3F, 0x7F, sounds);
                    self.seq.se_step += 1;
                }
            }
            2 => {
                if self.demo.time() >= 322.0 {
                    Self::play_se(0x40, 0x7F, sounds);
                    self.seq.se_step += 1;
                }
            }
            _ => {}
        }
        false
    }

    /// `ShenScene_Update`: the sequence and the camera, the objects, then the stage, the objects and the screen
    /// effects drawn.
    pub(crate) fn update(&mut self, libc: &mut LibcRandom, vu0: &mut Vu0Random) -> SceneFrame {
        let mut frame = SceneFrame::default();
        // ShenScene_RunSeq: once it has ended, the state stays ENDED.
        if self.seq.running && self.step_seq(vu0, &mut frame.sounds) {
            self.seq.running = false;
        }
        if !self.seq.running {
            self.set_state(STATE_ENDED);
        }
        if self.demo.is_active() {
            let view = if self.demo.fixed {
                Some(self.demo.update_fixed(libc))
            } else {
                self.demo.update(None, false, 1.0, libc)
            };
            if let Some(view) = view {
                self.view = view;
            }
        }
        for a in &mut self.actors {
            a.update();
        }
        self.time += 2.0;
        self.frame += 1;
        let view = &self.view;
        frame.stage = crate::stage_draw::draw_stage(&mut self.stage, self.stage_id, view);
        // The objects' rows and glow (`BtlObj_SetAlphaAdd`), then BtlObj_FinishVisibility's
        // BtlObj_UploadAlphaTable.
        for a in &mut self.actors {
            a.obj.alpha_add = a.alpha_add;
        }
        self.alpha_rows
            .update(self.actors.iter_mut().map(|a| &mut a.obj));
        let mut table = [0u8; 256];
        for a in &self.actors {
            a.obj.fill_alpha_row(&mut table, &a.model.alpha);
        }
        let mut groups = HashMap::new();
        for a in &self.actors {
            if a.obj.flags & OBJ_VISIBLE != 0 {
                crate::battle::draw_effect_model(
                    &a.model,
                    &a.obj,
                    self.light_dir,
                    self.ambient,
                    view,
                    &mut groups,
                );
            }
        }
        frame.models = ModelBatch::from_groups(groups);
        frame.obj_alpha = (!frame.models.is_empty()).then(|| crate::battle::ObjAlpha {
            table: Box::new(table),
        });
        // StgFx_DrawPost: the blur (no battle camera: the view drawn), then the shock waves stepped and drawn.
        frame.post = self.blurs.draw(0, &view.world2view, false);
        if self.warps.any() {
            self.warps
                .update(&[DrawCtx::new(view.world2screen, view.world2view)]);
            frame.post.extend(self.warps.draw(None));
        }
        // StgFx_DrawOverlay: ScrXfade_PostDraw and ScrXfade_Update.
        (frame.capture, frame.xfade_alpha) = self.xfade.draw_pass();
        frame
    }

    /// The glow each object is given (`BtlObj_SetAlphaAdd`): the dragon's, the balls'.
    #[cfg(test)]
    pub(crate) fn alpha_add(&self) -> [i32; 2] {
        [self.actors[0].alpha_add, self.actors[1].alpha_add]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each dragon's scene from the ISO in `rom/`, when it is there: its backdrop and models load, its two
    /// cameras play, the balls pulse and the dragon comes (READY) with the blur on, then the farewell ends it.
    #[test]
    fn real_scenes_summon_and_dismiss_each_dragon() {
        use bt3_data::Iso9660;
        use bt3_data::afs::{AfsArchive, partition_entry_for_id};

        let iso = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../rom/Dragon Ball Z - Budokai Tenkaichi 3 (USA) (En,Ja).iso");
        let Ok(mut image) = Iso9660::open(&iso) else {
            return;
        };
        let mut read = |id: u32| -> Result<Vec<u8>, String> {
            let location = partition_entry_for_id(id);
            let archive =
                AfsArchive::open(&mut image, format!("DATA/PZS3US{}.AFS", location.partition))
                    .map_err(|e| e.to_string())?;
            archive
                .read_entry(&mut image, location.index)
                .map_err(|e| e.to_string())
        };
        let mut libc = LibcRandom::new(1);
        let mut vu0 = Vu0Random::default();
        for dragon in 0..3 {
            let mut scene = ShenScene::new(&mut read, dragon).unwrap();
            assert!(
                scene.seq.cams.iter().all(Option::is_some),
                "dragon {dragon}"
            );
            let mut frames = 0;
            let mut pulsed = false;
            while scene.state() != STATE_READY && frames < 2000 {
                scene.fade.update();
                let out = scene.update(&mut libc, &mut vu0);
                assert!(!out.stage.is_empty());
                pulsed |= scene.alpha_add()[1] > 0;
                frames += 1;
            }
            assert_eq!(scene.state(), STATE_READY, "dragon {dragon}");
            assert!(pulsed);
            // The dragon is drawn; the shock wave and the blur run while the camera shakes.
            let out = scene.update(&mut libc, &mut vu0);
            assert!(!out.models.is_empty());
            assert!(!out.post.is_empty(), "dragon {dragon}");
            scene.set_state(STATE_LEAVE);
            let mut frames = 0;
            while scene.state() != STATE_ENDED && frames < 2000 {
                // Gfx_BeginFrame's Fade_UpdateAll.
                scene.fade.update();
                scene.update(&mut libc, &mut vu0);
                frames += 1;
            }
            assert_eq!(scene.state(), STATE_ENDED, "dragon {dragon}");
            assert!(scene.fade.is_done());
        }
    }
}
