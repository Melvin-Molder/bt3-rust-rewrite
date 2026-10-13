//! The menus' character model viewer (`ChrView`, src/ui/char_viewer.c) and its orbit camera (`OrbitCam`,
//! src/battle/orbit_cam.c): one character model at a time on a backdrop (stage file 0x197), playing its model
//! animation 7, seen from a camera on a sphere around it that pad 0's sticks turn and zoom.
//!
//! A new model is loaded by a job over a few frames into a buffer of its own; three frames after its object was
//! made the old one goes and the new one is shown (the original keeps two buffers for this). The viewer's scene
//! runs only its layer 3 (`BtlScene_Init(8)`), so the backdrop's stage effects of layer 0 (its water surface) are
//! not drawn, as in the original. Once a model is shown the backdrop's ambience runs (`StgAmb_Init`: the stage
//! number in its data is 5, a looping sound at a constant volume), until the model is taken off. Left out: the
//! stage's tint ramps (`StgFx_DrawPreNoCheck`, off here) and the fade slot `ChrView_Init` starts (the menu's own
//! fade covers it).

use std::collections::HashMap;

use bt3_data::gs::RgbaTexture;
use bt3_data::stage_model::StageModel;

use crate::battle::{FighterModel, ModelBatch, ModelSink, View};
use crate::btl_obj::Obj;
use crate::rng::LibcRandom;
use crate::stage_draw::StageBatch;

/// `CHRVIEW_FILE_STAGE`: the backdrop, stage file `STAGE_FILE + 0x26`.
const FILE_STAGE: u32 = 0x197;
/// `CHRVIEW_FILE_FIRST`: the model of character `c`, costume `n`: `0x590 + c * 10 + n` (+ 4 damaged).
const FILE_FIRST: u32 = 0x590;
/// `gChrView->anim`.
const ANIM: usize = 7;
/// `BOBJ_FLAG_VISIBLE` (part 0 of `BtlObj_SetPartVisible`) and the flag `ChrView_StepLoad` clears.
const OBJ_VISIBLE: u32 = 2;
const OBJ_HIDE_EVENT: u32 = 0x0200_0000;

/// `OrbitCam`: a camera on a sphere around `target`, looking at it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OrbitCam {
    /// x pitch, y yaw (kept in -pi..pi), z roll.
    angles: [f32; 3],
    target: [f32; 3],
    dist: f32,
    dist_min: f32,
    dist_max: f32,
    pitch_limit: f32,
    pad_locked: bool,
    /// The pitch is kept in -limit..0.
    floor_clamp: bool,
    eye: [f32; 3],
}

impl OrbitCam {
    /// `OrbitCam_Reset`.
    pub(crate) fn new() -> Self {
        Self {
            dist: 21.0,
            dist_min: 5.0,
            dist_max: 500.0,
            pitch_limit: std::f32::consts::FRAC_PI_2,
            target: [0.0, -8.29, 0.0],
            angles: [0.0, std::f32::consts::PI, 0.0],
            ..Self::default()
        }
    }

    /// `OrbitCam_Update(usePad)`: the left stick turns (yaw by x, pitch by y), the right stick's y zooms, each
    /// with a dead zone of 0.1; then the view from `target - direction * dist`.
    pub(crate) fn update(&mut self, left: [f32; 2], right_y: f32, use_pad: bool) -> View {
        let use_pad = use_pad && !self.pad_locked;
        let dead = |v: f32| {
            if use_pad && (v > 0.1 || v < -0.1) {
                v
            } else {
                0.0
            }
        };
        let (x, y, z) = (dead(left[0]), dead(left[1]), dead(right_y));
        let pi = std::f32::consts::PI;
        self.angles[1] += x * 0.1;
        if self.angles[1] < -pi {
            self.angles[1] += 2.0 * pi;
        }
        if self.angles[1] > pi {
            self.angles[1] -= 2.0 * pi;
        }
        self.angles[0] -= y * 0.1;
        if self.angles[0] < -self.pitch_limit {
            self.angles[0] = -self.pitch_limit;
        }
        let max = if self.floor_clamp {
            0.0
        } else {
            self.pitch_limit
        };
        if self.angles[0] > max {
            self.angles[0] = max;
        }
        self.dist = (self.dist + z * 0.5).max(self.dist_min).min(self.dist_max);
        let (sp, cp) = self.angles[0].sin_cos();
        let (sy, cy) = self.angles[1].sin_cos();
        let dir = [cp * sy, -sp, cp * cy];
        self.eye = std::array::from_fn(|k| self.target[k] - dir[k] * self.dist);
        View::new(self.eye, self.angles)
    }
}

/// The textures a model of the viewer binds (`ModelSink`).
#[derive(Default)]
struct Textures {
    base: Vec<RgbaTexture>,
    shade: Vec<RgbaTexture>,
    ids: Vec<RgbaTexture>,
    face: crate::btl_face::FaceTables,
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
        self.face.clone()
    }
}

/// `ChrViewSlot` with its object.
struct Shown {
    model: FighterModel,
    obj: Obj,
}

/// `ChrViewJob`.
struct Job {
    state: i32,
    count: i32,
    chara: i32,
    costume: i32,
    damaged: bool,
    slot: Option<Shown>,
}

/// What a frame of the viewer draws.
#[derive(Default)]
pub(crate) struct ViewerFrame {
    pub stage: Vec<StageBatch>,
    pub models: Vec<ModelBatch>,
    /// `BtlObj_BeginDraw`'s passes, while the model is drawn.
    pub obj_alpha: Option<crate::battle::ObjAlpha>,
}

/// `ChrView`.
pub(crate) struct ChrView {
    ramp: Vec<u8>,
    stage: StageModel,
    stage_id: i32,
    /// The stage number in the backdrop's data (`BtlStage_GetId`), which picks the ambience.
    stage_number: i32,
    water_level: f32,
    /// `gStgAmb`, and the stage sounds its handler would play (no bank is loaded for them here).
    amb: crate::stg_amb::StgAmb,
    amb_snd: crate::btl_snd::CharSound,
    /// The stage's textures, then the model's (from `textures`), as the renderer takes them.
    pub stage_textures: Vec<RgbaTexture>,
    textures: Textures,
    /// The objects' rows of the alpha table.
    alpha_rows: crate::btl_obj::AlphaRows,
    /// Bumped when `textures` starts over for a new model.
    pub model_rev: u32,
    light_dir: [f32; 3],
    cur: Option<Shown>,
    job: Option<Job>,
    loading: bool,
    visible: bool,
    stage_on: bool,
    zoom: bool,
    delay: i32,
    cam: OrbitCam,
    /// The character table's camera height and distance by character.
    target_y: Vec<f32>,
    dist: Vec<f32>,
}

impl ChrView {
    /// `ChrView_Init`: the backdrop (stage file 0x197, read at once), the light ramp, the orbit camera.
    pub(crate) fn new(
        read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>,
        face: crate::btl_face::FaceTables,
        chr: &bt3_data::chara_data::ChrTable,
    ) -> Result<Self, String> {
        let common = read(crate::battle::COMMON_LIGHT_FILE)?;
        let ramp = crate::battle::light_ramp(&common)?;
        let stage_id = (FILE_STAGE - crate::battle::STAGE_FILE) as i32;
        let loaded = crate::battle::load_stage(read, stage_id, false)?;
        // ChrView_Init's StgAmb_Term.
        let mut amb = crate::stg_amb::StgAmb::default();
        amb.stop();
        let light_dir = crate::battle::normalize(
            loaded
                .stage
                .light_dir
                .unwrap_or(crate::battle::DEFAULT_LIGHT),
        );
        Ok(Self {
            ramp,
            stage: loaded.model,
            stage_id,
            stage_number: loaded.stage.stage_id,
            water_level: loaded.stage.water_level.unwrap_or(0.0),
            amb,
            amb_snd: Default::default(),
            stage_textures: loaded.textures,
            textures: Textures {
                face,
                ..Textures::default()
            },
            alpha_rows: crate::btl_obj::AlphaRows::default(),
            model_rev: 0,
            light_dir,
            cur: None,
            job: None,
            loading: false,
            visible: false,
            stage_on: false,
            zoom: false,
            delay: 0,
            cam: OrbitCam::new(),
            target_y: chr.view_target_y.clone(),
            dist: chr.view_dist.clone(),
        })
    }

    /// The model's textures, as the renderer takes them.
    pub(crate) fn model_textures(&self) -> crate::battle::ModelTextures {
        (
            self.textures.base.clone(),
            self.textures.shade.clone(),
            self.textures.ids.clone(),
        )
    }

    /// What the ambience asked of the sound players since the last call.
    pub(crate) fn take_sounds(&mut self) -> Vec<crate::title::TitleSound> {
        std::mem::take(&mut self.amb.out)
    }

    /// `ChrView_IsVisible`.
    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    /// `ChrView_Show`: a new model is loaded and replaces the one on screen when it is ready.
    pub(crate) fn show(&mut self, chara: i32, costume: i32, damaged: bool) {
        if self.loading {
            self.cancel_load();
        }
        self.job = Some(Job {
            state: 0,
            count: 0,
            chara,
            costume,
            damaged,
            slot: None,
        });
    }

    /// `ChrView_Hide`: the model off the screen, and the ambience (`StgAmb_Term`).
    pub(crate) fn hide(&mut self) {
        self.stage_on = false;
        self.amb.stop();
        self.visible = false;
        if self.loading {
            self.cancel_load();
        }
        self.cur = None;
    }

    /// `ChrView_CancelLoad`.
    fn cancel_load(&mut self) {
        self.job = None;
        self.loading = false;
    }

    /// `ChrView_SetupCamera`: at the character table's height and distance (fixed unless zooming is allowed).
    fn setup_camera(&mut self, chara: i32) {
        let Some(i) = usize::try_from(chara).ok().filter(|&i| i < self.dist.len()) else {
            return;
        };
        let mut cam = OrbitCam::new();
        cam.floor_clamp = true;
        cam.target = [0.0, self.target_y[i], 0.0];
        cam.angles = [0.0, std::f32::consts::PI, 0.0];
        cam.dist = self.dist[i];
        (cam.dist_min, cam.dist_max) = if self.zoom {
            (3.0, 100.0)
        } else {
            (self.dist[i], self.dist[i])
        };
        cam.pitch_limit = std::f32::consts::FRAC_PI_4;
        self.cam = cam;
    }

    /// `Job_Run` of `ChrView_StepLoad`: the model read into a buffer and its object made (playing animation 7,
    /// looping, hidden), then three frames later shown in place of the old one.
    pub(crate) fn step_job(&mut self, read: &mut dyn FnMut(u32) -> Result<Vec<u8>, String>) {
        let Some(mut job) = self.job.take() else {
            return;
        };
        let mut state = job.state;
        loop {
            match state {
                0 => {
                    self.loading = true;
                    job.count += 1;
                    if job.count > self.delay {
                        state = 1;
                    }
                    if self.delay != 0 {
                        break;
                    }
                }
                1 => {
                    // BtlRes_LoadSingle into a free buffer: the viewer's textures start over.
                    let file = FILE_FIRST
                        + job.chara.max(0) as u32 * 10
                        + job.costume.max(0) as u32
                        + if job.damaged { 4 } else { 0 };
                    self.textures.base.clear();
                    self.textures.shade.clear();
                    self.model_rev += 1;
                    let loaded = read(file).and_then(|bytes| {
                        crate::battle::load_partner(&bytes, file, &self.ramp, &mut self.textures)
                    });
                    match loaded {
                        Ok(model) => {
                            let data = crate::battle::side_data(&model);
                            let obj = Obj::new(&data.obj, 0.0);
                            job.slot = Some(Shown { model, obj });
                            state = 2;
                        }
                        Err(_) => {
                            // A model that cannot be read is dropped, as a failed slot is.
                            self.loading = false;
                            return;
                        }
                    }
                    break;
                }
                2 => {
                    if let Some(s) = &mut job.slot {
                        let data = crate::battle::side_data(&s.model);
                        s.obj.step = 1.0;
                        s.obj.play(&data.obj, 0, ANIM, true);
                        s.obj.flags &= !OBJ_HIDE_EVENT;
                        s.obj.flags &= !OBJ_VISIBLE;
                    }
                    job.count = 0;
                    state = 3;
                }
                3 => {
                    let n = job.count;
                    job.count += 1;
                    if n >= 3 {
                        self.setup_camera(job.chara);
                        if let Some(mut s) = job.slot.take() {
                            s.obj.flags |= OBJ_VISIBLE;
                            self.cur = Some(s);
                        }
                        self.stage_on = true;
                        self.visible = true;
                        self.amb.start(self.stage_number);
                        self.loading = false;
                        return;
                    }
                    break;
                }
                _ => return,
            }
        }
        job.state = state;
        self.job = Some(job);
    }

    /// `ChrView_Update`: the camera from pad 0's sticks, the object (its looping animation and its face), then
    /// the backdrop and the model, while a model is shown.
    pub(crate) fn update(
        &mut self,
        left: [f32; 2],
        right_y: f32,
        libc: &mut LibcRandom,
    ) -> ViewerFrame {
        let view = self.cam.update(left, right_y, true);
        // BtlObj_UpdateAll: the model's animation (mode 2, looping), pose and face; the loading one's too.
        let objs = self
            .cur
            .iter_mut()
            .chain(self.job.as_mut().and_then(|j| j.slot.as_mut()));
        for s in objs {
            let data = crate::battle::side_data(&s.model);
            let length = s.obj.layers[0].length;
            if s.obj.layers[0].motion.is_some() {
                s.obj.prev_frame = s.obj.frame;
                s.obj.frame += s.obj.step;
                if length <= s.obj.frame {
                    s.obj.frame = 0.0;
                    s.obj.prev_frame = 0.0;
                }
            }
            s.obj.sample_pose(&data.obj);
            s.obj.calc_matrices(&data.obj);
            s.obj.update_face(libc);
        }
        let mut frame = ViewerFrame::default();
        if !self.visible {
            return frame;
        }
        // BtlStage_Update: StgAmb_Update.
        self.amb
            .update(Some(view.eye()), self.water_level, libc, &mut self.amb_snd);
        if self.stage_on {
            frame.stage = crate::stage_draw::draw_stage(&mut self.stage, self.stage_id, &view);
        }
        // The objects' rows, then BtlObj_FinishVisibility's BtlObj_UploadAlphaTable.
        let objs = self
            .cur
            .iter_mut()
            .chain(self.job.as_mut().and_then(|j| j.slot.as_mut()))
            .map(|s| &mut s.obj);
        self.alpha_rows.update(objs);
        let mut table = [0u8; 256];
        for s in self
            .cur
            .iter()
            .chain(self.job.as_ref().and_then(|j| j.slot.as_ref()))
        {
            s.obj.fill_alpha_row(&mut table, &s.model.alpha);
        }
        let mut groups = HashMap::new();
        if let Some(s) = &self.cur
            && s.obj.flags & OBJ_VISIBLE != 0
        {
            crate::battle::draw_model(&s.model, &s.obj, false, self.light_dir, &view, &mut groups);
        }
        frame.models = ModelBatch::from_groups(groups);
        frame.obj_alpha = (!frame.models.is_empty()).then(|| crate::battle::ObjAlpha {
            table: Box::new(table),
        });
        frame
    }
}
