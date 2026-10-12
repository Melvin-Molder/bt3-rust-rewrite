//! The partner object (`BtlPartner_*`, src/battle/btl_char_fx_2.c): the extra model a rush sequence or a fusion
//! brings in through the change queue's object requests (`BtlLoad_StepObject`: character file `chara * 10 +
//! costume` (+4 for the variant) below 0x100, object file 0xC1C + id from 0x100). It is a battle object of the
//! request's type (`BtlObj_InitFlags`: 1 plain, 2 like a fighter, 3 with flag 0x20000000), its own file's model,
//! textures, face, tables and chains, whose motions are the file's model animations. The fighter's manager pass
//! 6 puts it on the fighter and poses it; the actions start its animations (fighter animation numbers from
//! 0x19E: model animation `anim - 0x19E`), step them by hand and release it at the end.

use crate::btl_chr::Chr;
use crate::btl_obj::{Obj, ObjData};

/// The partner's files: the model as a fighter's is loaded (`load_fighter`), its tables and its model animations.
pub(crate) type PartnerModel = crate::battle::FighterModel;

/// What the partner object reads of its model (`BtlObj_BindTables`: the tables when the file has them).
pub(crate) fn obj_data(m: &PartnerModel) -> ObjData<'_> {
    ObjData {
        model: &m.model,
        motions: &m.motions,
        raw: &[],
        body: m.tables.body.as_ref(),
        chara: m.chara,
        height: m.tables.params.height(),
        face: m.face.as_ref(),
    }
}

/// `FxPartner` with its battle object.
#[derive(Clone, Debug)]
pub(crate) struct Partner {
    pub obj: Obj,
}

/// The file of an object request (`BtlLoad_StepObject`).
pub(crate) fn file_id(id: i32, costume: i32, variant: i32) -> u32 {
    if id < 0x100 {
        crate::battle::CHARA_FILE
            + id.max(0) as u32 * 10
            + costume.max(0) as u32
            + if variant != 0 { 4 } else { 0 }
    } else {
        0xC1C + id as u32
    }
}

/// `BtlObj_Create(type, ..)`: the object of the partner's model, hidden until its animation shows it, with the
/// flags of its type and the colour preset of its parameter block (row 0 of set 0 without one), its chains set up.
pub(crate) fn create(m: &PartnerModel, kind: i32) -> Partner {
    let data = obj_data(m);
    let mut obj = Obj::new(&data, m.tables.params.radius());
    let color = obj.flags & 0x3_0000;
    obj.flags = match kind {
        1 => 0x200_0006,
        0 | 2 => 0x1200_0016,
        _ => 0x2200_0016,
    } | color;
    if m.model.header.flags & 1 != 0 {
        obj.flags |= 8;
    }
    if m.tables.params.is_empty() {
        obj.set_color_preset(0, Some(0));
    } else {
        obj.set_color_preset(i32::from(m.tables.params.u8_at(3) as i8), None);
    }
    obj.init_chains(m.tables.chain.as_ref(), &data);
    Partner { obj }
}

/// `BtlPartner_Release`.
pub(crate) fn release(me: &mut Chr) {
    me.partner = None;
}

/// `BtlPartner_PlayAnim`: from fighter animation 0x19E on, the model animation `anim - 0x19E` from its start, by
/// hand; without frames the object hides; flags 0x20 / 0x40 clear and a talking mouth (modes 9..12) closes.
pub(crate) fn play_anim(me: &mut Chr, m: Option<&PartnerModel>, anim: i32) {
    let (Some(p), Some(m)) = (me.partner.as_deref_mut(), m) else {
        return;
    };
    if anim < 0x19E {
        return;
    }
    let obj = &mut p.obj;
    let index = (anim - 0x19E) as usize;
    if index < crate::eft_obj::MODEL_ANIMS {
        obj.play(&obj_data(m), 0, index, true);
    }
    if obj.layers[0].motion.is_none() || obj.layers[0].length == 0.0 {
        obj.flags &= !2;
    }
    obj.flags &= !(0x20 | 0x40);
    if matches!(obj.mouth_mode(), 9..=12) {
        obj.set_mouth(0);
    }
}

/// `BtlPartner_StepAnim`: the frame advances by the step, held at the end; true at the end (or without a partner).
pub(crate) fn step_anim(me: &mut Chr) -> bool {
    let Some(p) = me.partner.as_deref_mut() else {
        return true;
    };
    let obj = &mut p.obj;
    obj.prev_frame = obj.frame;
    obj.frame += obj.step;
    let end = obj.layers[0].length;
    if obj.frame >= end {
        obj.frame = end;
        return true;
    }
    false
}

/// `BtlPartner_SetFlag80`: flag 0x80 (the geometry on node 0x6F) on while its parameter block has flag
/// 0x200000 and `on`, else off, either only while it is drawn.
pub(crate) fn set_flag80(me: &mut Chr, m: Option<&PartnerModel>, on: bool) {
    let (Some(p), Some(m)) = (me.partner.as_deref_mut(), m) else {
        return;
    };
    let set = on && !m.tables.params.is_empty() && m.tables.params.flags() & 0x20_0000 != 0;
    let flags = &mut p.obj.flags;
    if set {
        if *flags & 0x82 != 0x82 {
            *flags |= 0x80;
        }
    } else if *flags & 0x82 == 0x82 {
        *flags &= !0x80;
    }
}

/// `BtlPartner_Update` (manager pass 6): the object on the fighter's place and turn, its pose sampled, its events
/// read (the face's held shapes among them), its matrices built around its chains; flag 0x2000000 clears.
pub(crate) fn update(me: &mut Chr, m: Option<&PartnerModel>, wind: [f32; 4]) {
    let (pos, rot) = (me.obj.pos, me.obj.rot);
    let (Some(p), Some(m)) = (me.partner.as_deref_mut(), m) else {
        return;
    };
    let data = obj_data(m);
    let obj = &mut p.obj;
    obj.pos = pos;
    obj.rot = rot;
    obj.xf_update();
    obj.save_node_positions(false);
    obj.sample_pose(&data);
    obj.update_events(&data);
    crate::btl_fx::obj_face_events(obj);
    obj.calc_matrices(&data);
    obj.update_chains(m.tables.chain.as_ref(), wind);
    obj.calc_matrices(&data);
    obj.flags &= !0x200_0000;
}

/// `BtlPartner_UpdateEvents` / `BtlPartner_HandleEvents` (manager pass 10): event 0x2000000 hides the object,
/// 0x4000000 shows it; events 0x20 / 0x40 and bits 43 / 44 set and clear its flags 0x20 / 0x40.
pub(crate) fn update_events(me: &mut Chr) {
    let Some(p) = me.partner.as_deref_mut() else {
        return;
    };
    let obj = &mut p.obj;
    if obj.test_event(0x200_0000) {
        obj.flags &= !2;
    }
    if obj.test_event(0x400_0000) {
        obj.flags |= 2;
    }
    // BtlFxObj_HandleFlagEvents.
    for (bit, flag, on) in [
        (0x20u64, 0x20u32, true),
        (0x40, 0x20, false),
        (1 << 43, 0x40, true),
        (1 << 44, 0x40, false),
    ] {
        if obj.test_event(bit) {
            if on {
                obj.flags |= flag;
            } else {
                obj.flags &= !flag;
            }
        }
    }
}
