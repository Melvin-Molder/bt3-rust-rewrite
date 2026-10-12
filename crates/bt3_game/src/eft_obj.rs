//! The effect model objects (`EftObj_*`, src/battle/eft_mesh.c): battle objects of type 1 made from a model pack
//! that is already in memory (a technique's pack entry), posed by a matrix the effect sets every frame or by one
//! of the model's own animations, shown or hidden by the effect, and drawn like the fighters (`BtlObjDraw_Draw`).
//! `BtlObj_UpdateAll` steps their animation and builds their node matrices once a frame.
//!
//! The battle object pool holds 12; with the two fighters, ten are left for these.

use bt3_data::model::CharaModel;
use bt3_data::motion::Motion;

use crate::battle::Mtx;
use crate::btl_obj::{Obj, ObjData};

/// `BObjMdl.modelAnims`: the model file's own animations (sections 0x1D..0x24).
pub(crate) const MODEL_ANIMS: usize = 8;
/// The object pool's ids less the fighters'.
const OBJ_MAX: usize = 10;

/// `BOBJ_PLAY_*`: how a model animation plays.
const PLAY_NONE: u8 = 0;
const PLAY_ONCE: u8 = 1;
const PLAY_LOOP: u8 = 2;

/// An effect model: the model, its textures by part and batch, and its animations.
#[derive(Clone, Debug)]
pub(crate) struct EffectModel {
    pub model: CharaModel,
    pub batch_textures: Vec<Vec<[usize; 2]>>,
    pub anims: Vec<Option<Motion>>,
}

impl EffectModel {
    /// What the object reads of the model (no body, no face; the animations by number).
    pub(crate) fn data(&self) -> ObjData<'_> {
        ObjData {
            model: &self.model,
            motions: &self.anims,
            raw: &[],
            body: None,
            chara: -1,
            height: 0.0,
            face: None,
        }
    }
}

/// Where an effect model is: fighter `side`'s technique slot `slot` (its pack's model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelRef {
    pub side: usize,
    pub slot: usize,
}

/// One effect object.
#[derive(Clone, Debug)]
pub(crate) struct EftObj {
    pub id: u32,
    pub model: ModelRef,
    pub obj: Obj,
    /// `anim.mode`.
    pub mode: u8,
}

/// The effect objects alive.
#[derive(Clone, Debug, Default)]
pub(crate) struct EftObjs {
    objs: Vec<EftObj>,
}

impl EftObjs {
    /// `EftObj_Create` under `handle`: a hidden object of the model (`BtlObj_InitFlags` of type 1: 0x2000006, 8
    /// for a fading model, and the default colour flag); false when the pool is full.
    pub(crate) fn create(&mut self, handle: u32, model: ModelRef, m: &EffectModel) -> bool {
        if self.objs.len() >= OBJ_MAX {
            return false;
        }
        let mut obj = Obj::new(&m.data(), 0.0);
        let color = obj.flags & 0x3_0000;
        obj.flags = 0x200_0006 | color;
        if m.model.header.flags & 1 != 0 {
            obj.flags |= 8;
        }
        // EftObj_SetVisible(obj, 0) right after (every user hides it at once).
        obj.flags &= !2;
        self.objs.push(EftObj {
            id: handle,
            model,
            obj,
            mode: PLAY_NONE,
        });
        true
    }

    /// `EftObj_Destroy`.
    pub(crate) fn destroy(&mut self, id: u32) {
        self.objs.retain(|o| o.id != id);
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut EftObj> {
        self.objs.iter_mut().find(|o| o.id == id)
    }

    /// `EftObj_SetMtx` (`BtlObjXf_SetMtx`): the object's world matrix.
    pub(crate) fn set_mtx(&mut self, id: u32, m: Mtx) {
        if let Some(o) = self.get_mut(id) {
            o.obj.mtx = m;
            o.obj.pos = [m[3][0], m[3][1], m[3][2]];
        }
    }

    /// `EftObj_SetVisible`: the object flag 2.
    pub(crate) fn set_visible(&mut self, id: u32, on: bool) {
        if let Some(o) = self.get_mut(id) {
            if on {
                o.obj.flags |= 2;
            } else {
                o.obj.flags &= !2;
            }
        }
    }

    /// `EftObj_PlayAnim` (`BtlObjAnim_PlayModel`): the model's animation `anim` from frame 0, once (1) or looping
    /// (2); a missing one stops the animation.
    pub(crate) fn play_anim(&mut self, id: u32, anim: usize, mode: u8, m: &EffectModel) {
        if anim >= MODEL_ANIMS {
            return;
        }
        if let Some(o) = self.get_mut(id) {
            o.obj.play(&m.data(), 0, anim, true);
            o.mode = if o.obj.layers[0].motion.is_some() {
                mode
            } else {
                PLAY_NONE
            };
        }
    }

    /// Where an object's model is.
    pub(crate) fn model_of(&self, id: u32) -> Option<ModelRef> {
        self.objs.iter().find(|o| o.id == id).map(|o| o.model)
    }

    /// `EftObj_IsAnimPlaying`.
    pub(crate) fn is_anim_playing(&self, id: u32) -> bool {
        self.objs
            .iter()
            .find(|o| o.id == id)
            .is_some_and(|o| o.mode != PLAY_NONE)
    }

    /// The objects, for drawing.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &EftObj> {
        self.objs.iter()
    }

    /// `BtlObj_UpdateAll` of the effect objects (not while paused): the animation stepped (`BtlObjAnim_Step`),
    /// the pose sampled and the node matrices built from the object's matrix. `model` finds an object's model.
    pub(crate) fn update<'a>(
        &mut self,
        paused: bool,
        model: impl Fn(ModelRef) -> Option<&'a EffectModel>,
    ) {
        if paused {
            return;
        }
        for o in &mut self.objs {
            let Some(m) = model(o.model) else {
                continue;
            };
            let data = m.data();
            let length = o.obj.layers[0].length;
            match o.mode {
                PLAY_ONCE => {
                    o.obj.prev_frame = o.obj.frame;
                    o.obj.frame += o.obj.step;
                    if length <= o.obj.frame {
                        o.mode = PLAY_NONE;
                        o.obj.frame = length;
                        o.obj.prev_frame = length;
                    }
                }
                PLAY_LOOP => {
                    o.obj.prev_frame = o.obj.frame;
                    o.obj.frame += o.obj.step;
                    if length <= o.obj.frame {
                        o.obj.frame = 0.0;
                        o.obj.prev_frame = 0.0;
                    }
                }
                _ => {}
            }
            o.obj.sample_pose(&data);
            o.obj.calc_matrices(&data);
        }
    }
}
