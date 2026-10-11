//! A fighter's battle object (src/battle/btl_obj_anim.c, the strike test of eft_detect.c, the box and sphere
//! tests of col_box.c / col_primitives.c): the two-layer animation player with its blend, the motion's events and
//! hit windows, the node matrices, the body boxes and this frame's strike volumes.
//!
//! The fighter code steps the player itself (`anim.manual`): it sets the frame, then the object samples the pose
//! (`BtlObjAnim_SamplePose`), reads the events (`BtlObjAnim_UpdateEvents`) and builds the matrices
//! (`BtlObjPose_CalcMatrices`). A throw's victim plays the thrower's motion retargeted to its own skeleton
//! (`BtlObjAnim_PlayFrom`). Not ported: the look-at rotations of nodes 0x2E / 0x2F, the jaw.

use std::f32::consts::PI;

use bt3_data::chara_data::BodyData;
use bt3_data::model::CharaModel;
use bt3_data::motion::{
    EV_HIT_BEGIN, EV_HIT_END, Motion, NODE_MAX, Track, quat_slerp, quat_unpack,
};

use crate::battle::{IDENTITY, Mtx, apply, mul};

/// `BOBJ_EVENT_MAX`: distinct events kept per frame.
const EVENT_MAX: usize = 8;
/// `BOBJ_ATK_NODE_MAX`.
const ATK_NODE_MAX: u32 = 19;
/// `BOBJ_EV_NODE`.
const EV_NODE: u64 = 0x4_0000_0000;
/// `BtlObj_GetDefaultAnimStep`.
pub(crate) const DEFAULT_STEP: f32 = 2.0;

/// What the object needs of its character's files.
pub(crate) struct ObjData<'a> {
    pub model: &'a CharaModel,
    /// By motion id, `None` for an empty section.
    pub motions: &'a [Option<Motion>],
    /// The motions other objects borrow (table flag 0x20000000), as decoded: before `BtlAnim_Play`'s table flags.
    pub raw: &'a [Option<Motion>],
    pub body: Option<&'a BodyData>,
    /// The model id `BtlAnim_HideModelNodes` tests (the character).
    pub chara: i32,
    /// `mdl.param->height` (parameter block +4): the scale of the vertical root motion.
    pub height: f32,
}

/// `BobjPart`: one node's local pose of this frame and its world matrix.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    pub pos: [f32; 3],
    pub rot: [f32; 4],
    from_pos: [f32; 3],
    from_rot: [f32; 4],
    pub mtx: Mtx,
    pub parent: Mtx,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            rot: [0.0, 0.0, 0.0, 1.0],
            from_pos: [0.0; 3],
            from_rot: [0.0, 0.0, 0.0, 1.0],
            mtx: IDENTITY,
            parent: IDENTITY,
        }
    }
}

/// `BobjAnimLayer`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layer {
    pub motion: Option<usize>,
    /// The motion is the object's borrowed copy (`BtlObjAnim_PlayFrom`), not one of its own.
    pub borrowed: bool,
    pub length: f32,
    cursors: [u16; NODE_MAX],
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            motion: None,
            borrowed: false,
            length: 0.0,
            cursors: [0; NODE_MAX],
        }
    }
}

/// `BobjHit`: the hit window open this frame and this frame's events.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hit {
    pub offset: [f32; 3],
    pub node_mask: u32,
    pub radius: [f32; 3],
    /// `hitCount`: hit windows of the motion.
    pub count: i8,
    /// `hitIndex`: the window open this frame (0..), else -1 minus the windows already begun.
    pub index: i8,
    pub event_count: usize,
    pub sphere: bool,
    /// The model node of the last `BOBJ_EV_NODE` event; 3 after a play.
    pub node: i32,
    pub scale: f32,
    pub events: [u64; EVENT_MAX],
    pub event_args: [u32; EVENT_MAX],
}

/// `BobjObb`: an oriented box placed by a matrix.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Obb {
    pub center: [f32; 3],
    pub half: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub world: [[f32; 3]; 8],
}

impl Obb {
    /// `ColObb_Init` + `ColObb_Update` (`BtlObjObb_SetMtx`): the corners in the box's space, then placed by `m`.
    fn new(half: [f32; 3], m: &Mtx) -> Self {
        let mut obb = Obb {
            center: [m[3][0], m[3][1], m[3][2]],
            half,
            axis: std::array::from_fn(|i| [m[i][0], m[i][1], m[i][2]]),
            world: [[0.0; 3]; 8],
        };
        for (i, w) in obb.world.iter_mut().enumerate() {
            let local = [
                if i & 1 == 0 { half[0] } else { -half[0] },
                if i & 2 == 0 { half[1] } else { -half[1] },
                if i & 4 == 0 { half[2] } else { -half[2] },
                1.0,
            ];
            let p = apply(m, local);
            *w = [p[0], p[1], p[2]];
        }
        obb
    }
}

/// An axis-aligned box, empty when min > max.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for Aabb {
    /// `StgAabb_SetEmpty`.
    fn default() -> Self {
        Self {
            min: [f32::MAX; 3],
            max: [-f32::MAX; 3],
        }
    }
}

impl Aabb {
    pub(crate) fn add(&mut self, p: [f32; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(p[i]);
            self.max[i] = self.max[i].max(p[i]);
        }
    }

    /// `ColBox_Overlaps`.
    pub fn overlaps(&self, o: &Aabb) -> bool {
        (0..3).all(|i| !(self.min[i] > o.max[i]) && !(self.max[i] < o.min[i]))
    }
}

/// A body box of this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BodyBox {
    pub obb: Obb,
    /// The part's sphere (`center`): centre and radius.
    pub center: [f32; 3],
    pub radius: f32,
}

/// `BobjBody` with the sphere records.
#[derive(Clone, Debug, Default)]
pub(crate) struct Body {
    pub boxes: Vec<BodyBox>,
    /// `body.box`: bounds of every part box.
    pub bounds: Aabb,
    /// `body.centerBox` (the object's `hitBox`): bounds of every part sphere.
    pub center_bounds: Aabb,
    /// The two body sphere records (centre, radius); `cur` / `prev` say which is this frame's and last frame's,
    /// `index` is the record the next swap makes current.
    pub sphere: [([f32; 3], f32); 2],
    pub cur: usize,
    pub prev: usize,
    pub index: usize,
    /// `body.radius`: the parameter block's, or twice the model's size[3].
    pub radius: f32,
    /// `curRadius`: the radius used against the stage, growing by 1 per frame up to `radius`, at least 4.
    pub cur_radius: f32,
    /// `reset` / `teleported`: no sweep from the previous position this frame.
    pub reset: bool,
    /// How far the stage pushed the object this frame (old - new).
    pub push: [f32; 3],
}

/// What the strikes and the stage leave in the object's work buffer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Work {
    pub atk_spheres: Vec<([f32; 3], f32)>,
    pub atk_obbs: Vec<Obb>,
    pub atk_bounds: Aabb,
    /// `hitFlags`: bit 0x18 + n my strike touched object n, bit 0x1C + n object n's strike touched me; the low
    /// bits are the stage contact word.
    pub hit_flags: u32,
    /// `contact`: the flags and plane of the polygon the body sphere was last pushed out of.
    pub contact_flags: u32,
    pub contact_nrm: [f32; 4],
    /// The ground under the fighter (`work->ground`).
    pub ground: Option<bt3_data::stage_col::Ground>,
    pub ground_pos: [f32; 3],
}

/// `BtlObj` as far as a fighter's simulation uses it.
#[derive(Clone, Debug)]
pub(crate) struct Obj {
    /// Per model part (skeleton record) in file order.
    pub nodes: Vec<Node>,
    /// `BtlObj_GetNode`: part index by node id.
    node_index: [Option<u16>; 0x80],
    pub layers: [Layer; 2],
    pub frame: f32,
    pub prev_frame: f32,
    pub step: f32,
    pub blend: f32,
    pub blend_step: f32,
    pub mix: f32,
    /// `anim.rate` (object + 0xCB8).
    pub rate: f32,
    pub hit: Hit,
    /// `xf`: position, rotation, scale, the matrix and node 0's world position (`outPos`).
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub scale: f32,
    pub mtx: Mtx,
    pub root_pos: [f32; 3],
    /// `look.node2`: the lean quaternion multiplied onto node 2.
    pub lean: [f32; 4],
    /// Object flags (+0xA40).
    pub flags: u32,
    /// `area`: the zone the object is in.
    pub area: i32,
    pub body: Body,
    pub work: Work,
    /// Layer 0's borrowed motion (`BtlObjAnim_PlayFrom`).
    pub borrowed: Option<Motion>,
}

impl Obj {
    pub(crate) fn new(data: &ObjData, radius: f32) -> Self {
        let mut node_index = [None; 0x80];
        for (i, part) in data.model.parts.iter().enumerate() {
            if let Some(slot) = node_index.get_mut(usize::from(part.node)) {
                *slot = Some(i as u16);
            }
        }
        let size3 = data.model.header.size[3];
        // BtlObjBody_Init: the parameter block's radius, or twice the model's size[3].
        let radius = if radius <= 0.0 { size3 + size3 } else { radius };
        let boxes = data
            .body
            .map_or(Vec::new(), |b| vec![BodyBox::default(); b.parts.len()]);
        Self {
            nodes: vec![Node::default(); data.model.parts.len()],
            node_index,
            layers: [Layer::default(); 2],
            frame: 0.0,
            prev_frame: 0.0,
            step: DEFAULT_STEP,
            blend: 0.0,
            blend_step: 0.0,
            mix: 0.0,
            rate: 1.0,
            hit: Hit {
                scale: 1.0,
                ..Hit::default()
            },
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            mtx: IDENTITY,
            root_pos: [0.0; 3],
            lean: [0.0, 0.0, 0.0, 1.0],
            flags: 0,
            area: -1,
            // BtlObjBody_Init: curRadius 4.1, then BtlObjBody_Reset (index 1, prev 0, cur 1).
            body: Body {
                boxes,
                radius,
                cur_radius: 4.1,
                index: 1,
                prev: 0,
                cur: 1,
                reset: true,
                ..Body::default()
            },
            work: Work::default(),
            borrowed: None,
        }
    }

    /// `BtlObj_GetNode`: the part of node `id`.
    pub(crate) fn node(&self, id: u32) -> Option<&Node> {
        let i = (*self.node_index.get(id as usize)?)?;
        self.nodes.get(usize::from(i))
    }

    /// `BtlCharApi_GetNodePos`: node `id`'s world position (the object's position without the node).
    pub(crate) fn node_pos(&self, id: u32) -> [f32; 3] {
        self.node(id)
            .map_or(self.pos, |n| [n.mtx[3][0], n.mtx[3][1], n.mtx[3][2]])
    }

    /// `BtlObjAnim_Play`: layer 0 with `reset` restarts at frame 0, else keeps the frame (clamped to the length).
    /// An empty motion leaves the layer without data.
    pub(crate) fn play(&mut self, data: &ObjData, layer: usize, motion: usize, reset: bool) {
        let m = data.motions.get(motion).and_then(Option::as_ref);
        let l = &mut self.layers[layer];
        l.motion = m.map(|_| motion);
        l.borrowed = false;
        let Some(m) = m else {
            return;
        };
        l.length = f32::from(m.length);
        l.cursors = [0; NODE_MAX];
        if layer == 0 {
            self.hit.node = 3;
            if reset {
                self.frame = 0.0;
                self.prev_frame = 0.0;
            } else {
                self.frame = self.frame.min(l.length);
                self.prev_frame = self.prev_frame.min(l.length);
            }
        }
    }

    /// `BtlObjAnim_PlayFrom` (`useTable` 0): layer 0 plays a copy of the other object's motion moved to this
    /// skeleton (`BtlObjAnim_Retarget`: each translation key less the other's rest translation of the node, y scaled
    /// by the height ratio, plus this one's rest), then `BtlAnim_Play`'s table flags and hidden nodes.
    pub(crate) fn play_from(
        &mut self,
        data: &ObjData,
        other: &ObjData,
        motion: usize,
        table_flags: u32,
    ) {
        self.hit.node = 3;
        let Some(src) = other.raw.get(motion).and_then(Option::as_ref) else {
            self.layers[0].motion = None;
            self.layers[0].borrowed = false;
            return;
        };
        let mut m = src.clone();
        let scale = data.height / other.height;
        for bone in &data.model.parts {
            let id = usize::from(bone.node);
            if id >= NODE_MAX {
                continue;
            }
            let Some(Some(Track::Full(keys))) = m.tracks.get_mut(id) else {
                continue;
            };
            let Some(from) = other.model.parts.iter().find(|p| usize::from(p.node) == id) else {
                continue;
            };
            for key in keys.iter_mut() {
                let p = &mut key.1;
                for k in 0..3 {
                    p[k] -= from.rest[k];
                }
                p[1] *= scale;
                for k in 0..3 {
                    p[k] += bone.rest[k];
                }
            }
        }
        // BtlAnim_HideModelNode10, then BtlAnim_ApplyTableFlags and BtlAnim_HideModelNodes on the copy.
        if data.chara == 0x78
            && let Some(Some(t)) = m.tracks.get_mut(0x10)
        {
            match t {
                Track::Rot(keys) => keys.iter_mut().for_each(|k| k.1 = PACKED_IDENTITY_ROT),
                Track::Full(keys) => keys.iter_mut().for_each(|k| k.2 = PACKED_IDENTITY_ROT),
            }
        }
        m.apply_table_flags(table_flags);
        m.hide_model_nodes(data.chara);
        let l = &mut self.layers[0];
        l.motion = Some(motion);
        l.borrowed = true;
        l.length = f32::from(m.length);
        l.cursors = [0; NODE_MAX];
        self.frame = 0.0;
        self.prev_frame = 0.0;
        self.borrowed = Some(m);
    }

    /// The yaw of the first key of layer 0's root rotation track (`BtlActThrow_TurnByRootYaw`); `None` without one.
    pub(crate) fn root_key_yaw(&self, data: &ObjData) -> Option<f32> {
        let m = layer_motion(&self.layers[0], self.borrowed.as_ref(), data)?;
        let q = match m.tracks.first()?.as_ref()? {
            Track::Rot(keys) => keys.first().map(|k| k.1),
            Track::Full(keys) => keys.first().map(|k| k.2),
        };
        Some(q.map_or(0.0, |q| quat_to_euler(quat_unpack(q))[1]))
    }

    /// `BtlObjAnim_PromoteLayer`: layer 1 becomes layer 0 at frame 0.
    pub(crate) fn promote_layer(&mut self) {
        self.layers[0] = self.layers[1];
        self.layers[0].cursors = [0; NODE_MAX];
        self.frame = 0.0;
        self.prev_frame = 0.0;
    }

    /// `BtlObjAnim_StartBlend`: remembers every node's pose to blend from over `seconds`.
    pub(crate) fn start_blend(&mut self, seconds: f32) {
        if !(1.0 / 30.0 < seconds) {
            self.blend = 0.0;
            self.blend_step = 1.0;
            return;
        }
        self.blend = 1.0;
        self.blend_step = 1.0 / (seconds * 30.0);
        for n in &mut self.nodes {
            n.from_pos = n.pos;
            n.from_rot = n.rot;
        }
    }

    /// `BtlObjAnim_SamplePose`: every node's local translation and rotation: layer 0, layer 1 mixed in by `mix`
    /// (on the same relative frame), then the blend from the remembered pose (not on node 0).
    pub(crate) fn sample_pose(&mut self, data: &ObjData) {
        let Some(m0) = layer_motion(&self.layers[0], self.borrowed.as_ref(), data) else {
            return;
        };
        let m1 = layer_motion(&self.layers[1], self.borrowed.as_ref(), data);
        let b = self.blend;
        let w = if 0.0 < b {
            b * b * b * -2.0 + b * b * 3.0
        } else {
            0.0
        };
        for (part, node) in data.model.parts.iter().zip(&mut self.nodes) {
            let id = usize::from(part.node);
            let rest = [part.rest[0], part.rest[1], part.rest[2]];
            if id >= NODE_MAX {
                node.pos = rest;
                node.rot = [0.0, 0.0, 0.0, 1.0];
                continue;
            }
            match m0.tracks.get(id).and_then(Option::as_ref) {
                Some(track) => {
                    let (pos, rot) = track.sample(&mut self.layers[0].cursors[id], self.frame);
                    node.pos = pos.unwrap_or(rest);
                    node.rot = rot;
                }
                None => {
                    node.pos = rest;
                    node.rot = [0.0, 0.0, 0.0, 1.0];
                }
            }
            if let Some(m1) = m1
                && 0.0 < self.mix
            {
                let (l0, l1) = (self.layers[0].length, self.layers[1].length);
                let mut frame = self.frame;
                if l0 != l1 {
                    frame = (frame * (l1 / l0)).min(l1);
                }
                if let Some(track) = m1.tracks.get(id).and_then(Option::as_ref) {
                    let (pos, rot) = track.sample(&mut self.layers[1].cursors[id], frame);
                    let pos = pos.unwrap_or(rest);
                    // Vec4_Lerp(dst, pos, dst, mix): dst = pos * mix + dst * (1 - mix).
                    node.pos =
                        std::array::from_fn(|k| pos[k] * self.mix + node.pos[k] * (1.0 - self.mix));
                    node.rot = quat_slerp(node.rot, rot, self.mix);
                }
            }
            if id != 0 && 0.0 < w {
                node.pos = std::array::from_fn(|k| node.from_pos[k] * w + node.pos[k] * (1.0 - w));
                node.rot = quat_slerp(node.rot, node.from_rot, w);
            }
        }
    }

    /// `BtlObjAnim_UpdateEvents`: the hit window open in (previous frame, frame], how many windows the motion
    /// has, and up to eight distinct events of the interval.
    pub(crate) fn update_events(&mut self, data: &ObjData) {
        let hit = &mut self.hit;
        hit.index = -1;
        hit.node_mask = 0;
        hit.count = 0;
        let Some(m) = layer_motion(&self.layers[0], self.borrowed.as_ref(), data) else {
            return;
        };
        let cur = (self.frame + 1.0 / 60.0) as i32;
        let mut prev = (self.prev_frame + 1.0 / 60.0) as i32;
        if cur == prev {
            prev -= 1;
        }
        let (mut begin, mut arg, mut sphere, mut shape) = (0i32, 0u32, false, None);
        let (mut begins, mut ends) = (0, 0);
        for e in &m.events {
            if e.attr as i32 & EV_HIT_BEGIN as i32 != 0 {
                begin = i32::from(e.frame);
                arg = e.arg;
                sphere = (e.attr >> 32) & 1 != 0;
                shape = e.shape;
                begins += 1;
            }
            if e.attr & EV_HIT_END != 0 {
                ends += 1;
                let end = i32::from(e.frame);
                if begins == ends {
                    hit.count += 1;
                    if hit.index < 0 && !(cur < begin) && prev < end {
                        let s = shape.unwrap_or_default();
                        hit.offset = [s[0], s[1], s[2]];
                        hit.radius = [s[3] * hit.scale, s[4] * hit.scale, s[5] * hit.scale];
                        hit.index = (ends - 1) as i8;
                        hit.node_mask = arg;
                        hit.sphere = sphere;
                    }
                }
            }
        }
        if hit.count > 0 && hit.index < 0 {
            hit.index = -1;
            for e in &m.events {
                if e.attr as i32 & EV_HIT_BEGIN as i32 != 0 {
                    if cur < i32::from(e.frame) {
                        break;
                    }
                    hit.index -= 1;
                }
            }
        }
        let mut n = 0;
        for e in &m.events {
            let f = i32::from(e.frame);
            let inside = if prev > 0 {
                !(cur < f) && prev < f
            } else {
                !(cur < f) && !(f < prev)
            };
            if !inside || n >= EVENT_MAX {
                continue;
            }
            match hit.events[..n].iter().position(|&a| a == e.attr) {
                Some(k) => hit.event_args[k] |= e.arg,
                None => {
                    hit.events[n] = e.attr;
                    hit.event_args[n] = e.arg;
                    n += 1;
                }
            }
        }
        hit.event_count = n;
        if self.test_event(EV_NODE) {
            self.hit.node = mask_to_node(self.event_arg(EV_NODE));
        }
    }

    /// `BtlObjAnim_TestEvent`: an event with any of `mask`'s attribute bits fell in this frame's interval.
    pub(crate) fn test_event(&self, mask: u64) -> bool {
        self.hit.events[..self.hit.event_count]
            .iter()
            .any(|&e| e & mask != 0)
    }

    /// `BtlObjAnim_GetEventArg`.
    pub(crate) fn event_arg(&self, mask: u64) -> u32 {
        (0..self.hit.event_count)
            .filter(|&i| self.hit.events[i] & mask != 0)
            .fold(0, |a, i| a | self.hit.event_args[i])
    }

    /// `BtlObjAnim_QueryEvent`: of layer `layer`'s events with any bit of `mask`: 0 frame of the first, 1 of the
    /// first still ahead, 2 of the last, 3 how many, 4 how many ahead, 5 how many reached, 6 / 7 / 8 the arguments
    /// of 0 / 1 / 2. Frames are -1 and arguments 0 when there is none.
    pub(crate) fn query_event(&self, data: &ObjData, mask: u64, layer: usize, what: u32) -> i32 {
        let Some(m) = layer_motion(&self.layers[layer], self.borrowed.as_ref(), data) else {
            return 0;
        };
        let (mut first, mut next, mut last) = (-1, -1, -1);
        let (mut first_arg, mut next_arg, mut last_arg) = (0u32, 0u32, 0u32);
        let (mut count, mut ahead, mut reached) = (0, 0, 0);
        for e in m.events.iter().filter(|e| e.attr & mask != 0) {
            let f = i32::from(e.frame);
            if first < 0 {
                first = f;
                first_arg = e.arg;
            }
            if next < 0 && self.frame < f as f32 {
                next = f;
                next_arg = e.arg;
            }
            last = f;
            count += 1;
            last_arg = e.arg;
            if self.frame < last as f32 {
                ahead += 1;
            }
            if last as f32 <= self.frame {
                reached += 1;
            }
        }
        match what {
            0 => first,
            1 => next,
            2 => last,
            3 => count,
            4 => ahead,
            5 => reached,
            6 => first_arg as i32,
            7 => next_arg as i32,
            8 => last_arg as i32,
            _ => 0,
        }
    }

    /// `BtlObjXf_Update`: scale, rotation Z X Y with the yaw turned by pi (the model faces -Z), translation.
    pub(crate) fn xf_update(&mut self) {
        let mut yaw = self.rot[1] + PI;
        if PI < yaw {
            yaw -= 2.0 * PI;
        }
        let s = self.scale;
        let scale = [
            [s, 0.0, 0.0, 0.0],
            [0.0, s, 0.0, 0.0],
            [0.0, 0.0, s, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut m = mul(
            &mul(&mul(&scale, &rot_z(self.rot[2])), &rot_x(self.rot[0])),
            &rot_y(yaw),
        );
        m[3] = [self.pos[0], self.pos[1], self.pos[2], 1.0];
        self.mtx = m;
    }

    /// `BtlObjPose_CalcMatrices`: world matrix of every node, parents first; node 0's world position is kept.
    /// Node 2 takes the lean rotation on top of its own.
    pub(crate) fn calc_matrices(&mut self, data: &ObjData) {
        let mut stack = Vec::new();
        let mut current = self.mtx;
        for (part, node) in data.model.parts.iter().zip(&mut self.nodes) {
            stack.push(current);
            node.parent = current;
            let q = if part.node == 2 {
                quat_mul(self.lean, node.rot)
            } else {
                node.rot
            };
            let mut local = quat_to_mtx(q);
            local[3] = [node.pos[0], node.pos[1], node.pos[2], 1.0];
            node.mtx = mul(&local, &current);
            current = node.mtx;
            for _ in 0..part.pop {
                current = stack.pop().unwrap_or(self.mtx);
            }
        }
        if let Some(n) = self.node(0) {
            self.root_pos = [n.mtx[3][0], n.mtx[3][1], n.mtx[3][2]];
        }
    }

    /// `BtlObjBody_UpdateSphere`: centred on node 0 in x / z, at node 0x11's height minus 1.2, never closer than
    /// radius + 1.2 to node 0 (+Y is down).
    pub(crate) fn update_body_sphere(&mut self) {
        let (Some(n0), Some(n11)) = (self.node(0).copied(), self.node(0x11).copied()) else {
            return;
        };
        let y = n0.mtx[3][1];
        let r = self.body.radius;
        let mut c = [n0.mtx[3][0], n0.mtx[3][1], n0.mtx[3][2]];
        c[1] = n11.mtx[3][1] - 1.2;
        if y - 1.2 < c[1] + r {
            c[1] = y - r - 1.2;
        }
        self.body.sphere[self.body.cur] = (c, r);
    }

    /// `BtlObjBody_Swap` (in `BtlObjBody_BeginFrame`): the record at `index` becomes current.
    pub(crate) fn swap_body_sphere(&mut self) {
        let b = &mut self.body;
        let old = b.index;
        b.cur = old;
        b.prev = old ^ 1;
        b.index = old ^ 1;
    }

    /// `BtlObjBody_Warp`: both sphere records at `pos`, teleported (a fighter's body always exists).
    pub(crate) fn warp_body(&mut self, pos: [f32; 3]) {
        self.body.sphere[0].0 = pos;
        self.body.sphere[1].0 = pos;
        self.reset_body();
    }

    /// `BtlObjBody_Reset`: marks the body as teleported; the other record becomes current.
    pub(crate) fn reset_body(&mut self) {
        let b = &mut self.body;
        let old = b.index;
        b.reset = true;
        b.index = old ^ 1;
        b.prev = old;
        b.cur = b.index;
    }

    pub(crate) fn body_sphere(&self) -> ([f32; 3], f32) {
        self.body.sphere[self.body.cur]
    }

    /// `BtlObjHit_BuildVolumes`: the body part boxes from their node matrices, then one sphere or box per attack
    /// node of the open hit window, and their bounds.
    pub(crate) fn build_volumes(&mut self, data: &ObjData) {
        if let Some(body) = data.body {
            let mut bounds = Aabb::default();
            let mut center_bounds = Aabb::default();
            for (part, out) in body.parts.iter().zip(&mut self.body.boxes) {
                let Some(i) = self
                    .node_index
                    .get(usize::from(part.node))
                    .copied()
                    .flatten()
                else {
                    continue;
                };
                let origin = data.model.parts[usize::from(i)].offset_a;
                let m = self.nodes[usize::from(i)].mtx;
                let local = [
                    part.offset[0] - origin[0],
                    part.offset[1] - origin[1],
                    part.offset[2] - origin[2],
                    1.0,
                ];
                let c = apply(&m, local);
                let mut placed = m;
                placed[3] = [c[0], c[1], c[2], 1.0];
                out.obb = Obb::new(part.half, &placed);
                out.center = [c[0], c[1], c[2]];
                out.radius = part.radius;
                for p in out.obb.world {
                    bounds.add(p);
                }
                // ColBounds_AddSphere.
                let r = part.radius;
                center_bounds.add([c[0] + r, c[1] + r, c[2] + r]);
                center_bounds.add([c[0] - r, c[1] - r, c[2] - r]);
            }
            self.body.bounds = bounds;
            self.body.center_bounds = center_bounds;
        }
        let work = &mut self.work;
        work.atk_spheres.clear();
        work.atk_obbs.clear();
        work.hit_flags &= 0xFF_FFFF;
        work.atk_bounds = Aabb::default();
        if self.hit.index < 0 {
            return;
        }
        for code in 0..ATK_NODE_MAX {
            if self.hit.node_mask & (1 << code) == 0 {
                continue;
            }
            let Some(m) = node_by_code(code)
                .and_then(|id| self.node_index.get(id as usize).copied().flatten())
                .map(|i| self.nodes[usize::from(i)].mtx)
            else {
                continue;
            };
            let o = self.hit.offset;
            let c = apply(&m, [o[0], o[1], o[2], 1.0]);
            let c = [c[0], c[1], c[2]];
            if self.hit.sphere {
                let r = self.hit.radius[0];
                work.atk_spheres.push((c, r));
                work.atk_bounds.add([c[0] + r, c[1] + r, c[2] + r]);
                work.atk_bounds.add([c[0] - r, c[1] - r, c[2] - r]);
            } else {
                let mut placed = m;
                placed[3] = [c[0], c[1], c[2], 1.0];
                let obb = Obb::new(self.hit.radius, &placed);
                for p in obb.world {
                    work.atk_bounds.add(p);
                }
                work.atk_obbs.push(obb);
            }
        }
    }
}

/// `BtlBodyHit_Test`: whether `atk`'s strike volumes touch `def`'s body boxes; records it in both objects.
pub(crate) fn body_hit_test(atk: &mut Obj, atk_id: u32, def: &mut Obj, def_id: u32) -> bool {
    if atk.hit.index < 0 || def.body.boxes.is_empty() {
        return false;
    }
    if !atk.work.atk_bounds.overlaps(&def.body.bounds) {
        return false;
    }
    let touched = atk.work.atk_spheres.iter().any(|&(c, r)| {
        def.body
            .boxes
            .iter()
            .any(|b| sphere_contacts_obb(c, r, &b.obb))
    }) || atk
        .work
        .atk_obbs
        .iter()
        .any(|a| def.body.boxes.iter().any(|b| obb_overlaps(a, &b.obb)));
    if touched {
        atk.work.hit_flags |= 1 << (def_id + 0x18);
        def.work.hit_flags |= 1 << (atk_id + 0x1C);
    }
    touched
}

/// `ColSphere_ContactObb`, yes or no.
pub(crate) fn sphere_contacts_obb(c: [f32; 3], r: f32, obb: &Obb) -> bool {
    let d = [
        c[0] - obb.center[0],
        c[1] - obb.center[1],
        c[2] - obb.center[2],
    ];
    let dot = |a: [f32; 3]| d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
    let p = [
        dot(obb.axis[0]).abs() - obb.half[0],
        dot(obb.axis[1]).abs() - obb.half[1],
        dot(obb.axis[2]).abs() - obb.half[2],
    ];
    if p[0] > r || p[1] > r || p[2] > r {
        return false;
    }
    let rr = r * r;
    if p[0] < 0.0 {
        if p[1] < 0.0 || p[2] < 0.0 {
            return true;
        }
        p[1] * p[1] + p[2] * p[2] <= rr
    } else if p[1] < 0.0 {
        if p[2] < 0.0 {
            return true;
        }
        p[0] * p[0] + p[2] * p[2] <= rr
    } else if p[2] < 0.0 {
        p[0] * p[0] + p[1] * p[1] <= rr
    } else {
        p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= rr
    }
}

/// `ColObb_Overlaps`: the fifteen separating axes, touching counts.
pub(crate) fn obb_overlaps(a: &Obb, b: &Obb) -> bool {
    let dot = |x: [f32; 3], y: [f32; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let (ha, hb) = (a.half, b.half);
    let diff = [
        a.center[0] - b.center[0],
        a.center[1] - b.center[1],
        a.center[2] - b.center[2],
    ];
    let r: [[f32; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|j| dot(a.axis[i], b.axis[j])));
    let ar: [[f32; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| r[i][j].abs()));
    let d: [f32; 3] = std::array::from_fn(|i| dot(a.axis[i], diff));
    for i in 0..3 {
        let rb = hb[0] * ar[i][0] + hb[1] * ar[i][1] + hb[2] * ar[i][2];
        if d[i].abs() > ha[i] + rb {
            return false;
        }
    }
    for j in 0..3 {
        let t = dot(b.axis[j], diff).abs();
        let ra = ha[0] * ar[0][j] + ha[1] * ar[1][j] + ha[2] * ar[2][j];
        if t > ra + hb[j] {
            return false;
        }
    }
    // The nine edge axes, as the original writes them out.
    let edges = [
        (
            d[2] * r[1][0] - d[1] * r[2][0],
            ha[1] * ar[2][0] + ha[2] * ar[1][0],
            hb[1] * ar[0][2] + hb[2] * ar[0][1],
        ),
        (
            d[2] * r[1][1] - d[1] * r[2][1],
            ha[1] * ar[2][1] + ha[2] * ar[1][1],
            hb[0] * ar[0][2] + hb[2] * ar[0][0],
        ),
        (
            d[2] * r[1][2] - d[1] * r[2][2],
            ha[1] * ar[2][2] + ha[2] * ar[1][2],
            hb[0] * ar[0][1] + hb[1] * ar[0][0],
        ),
        (
            d[0] * r[2][0] - d[2] * r[0][0],
            ha[0] * ar[2][0] + ha[2] * ar[0][0],
            hb[1] * ar[1][2] + hb[2] * ar[1][1],
        ),
        (
            d[0] * r[2][1] - d[2] * r[0][1],
            ha[0] * ar[2][1] + ha[2] * ar[0][1],
            hb[0] * ar[1][2] + hb[2] * ar[1][0],
        ),
        (
            d[0] * r[2][2] - d[2] * r[0][2],
            ha[0] * ar[2][2] + ha[2] * ar[0][2],
            hb[0] * ar[1][1] + hb[1] * ar[1][0],
        ),
        (
            d[1] * r[0][0] - d[0] * r[1][0],
            ha[0] * ar[1][0] + ha[1] * ar[0][0],
            hb[1] * ar[2][2] + hb[2] * ar[2][1],
        ),
        (
            d[1] * r[0][1] - d[0] * r[1][1],
            ha[0] * ar[1][1] + ha[1] * ar[0][1],
            hb[0] * ar[2][2] + hb[2] * ar[2][0],
        ),
        (
            d[1] * r[0][2] - d[0] * r[1][2],
            ha[0] * ar[1][2] + ha[1] * ar[0][2],
            hb[0] * ar[2][1] + hb[1] * ar[2][0],
        ),
    ];
    edges.iter().all(|&(t, ra, rb)| t.abs() <= ra + rb)
}

/// `BtlObj_GetNodeByCode`: attack node code 0..18 -> model node.
fn node_by_code(code: u32) -> Option<u32> {
    const NODES: [u32; 19] = [
        3, 7, 0xA, 0xB, 0xE, 0xF, 0x13, 0x14, 0x15, 0x21, 0x22, 0x23, 0x2E, 0x40, 0x41, 0, 0x1F,
        0x2D, 0x36,
    ];
    NODES.get(code as usize).copied()
}

/// `BtlObjAnim_MaskToNode`: the lowest set bit of an event argument, in the original's priority order.
/// `BtlObj_GetNodeSide`: 0 / 1 the two sides of the body, 2 the centre, 3 the ids from 0x35.
pub(crate) fn node_side(node: i32) -> i32 {
    match node {
        ..8 => 2,
        ..0xC => 1,
        ..0x10 => 0,
        ..0x12 => 2,
        ..0x20 => 1,
        ..0x2E => 0,
        ..0x35 => 2,
        _ => 3,
    }
}

/// The motion a layer plays: its own table's, or the borrowed copy.
fn layer_motion<'a>(
    layer: &Layer,
    borrowed: Option<&'a Motion>,
    data: &'a ObjData,
) -> Option<&'a Motion> {
    if layer.borrowed {
        return borrowed;
    }
    layer
        .motion
        .and_then(|id| data.motions.get(id))
        .and_then(Option::as_ref)
}

/// The packed identity rotation (`BtlObjAnim_ClearNodeRot`).
const PACKED_IDENTITY_ROT: u64 = bt3_data::motion::PACKED_IDENTITY;

pub(crate) fn mask_to_node(mask: u32) -> i32 {
    const ORDER: [(u32, i32); 19] = [
        (1, 3),
        (2, 4),
        (4, 10),
        (8, 11),
        (0x40, 0x13),
        (0x80, 0x14),
        (0x100, 0x15),
        (0x10, 0xE),
        (0x20, 0xF),
        (0x200, 0x21),
        (0x400, 0x22),
        (0x800, 0x23),
        (0x1000, 0x2E),
        (0x2000, 0x40),
        (0x4000, 0x41),
        (0x8000, 0),
        (0x1_0000, 0x1F),
        (0x2_0000, 0x2D),
        (0x4_0000, 0x36),
    ];
    ORDER
        .iter()
        .find(|(bit, _)| mask & bit != 0)
        .map_or(0, |&(_, node)| node)
}

/// `Quat_Mul`.
pub(crate) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `Quat_FromAxisAngle` for a unit axis.
fn quat_axis(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (s, c) = (angle * 0.5).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}

/// `Quat_FromEuler`: qy * (qx * qz).
pub(crate) fn quat_from_euler(a: [f32; 3]) -> [f32; 4] {
    let qx = quat_axis([1.0, 0.0, 0.0], a[0]);
    let qy = quat_axis([0.0, 1.0, 0.0], a[1]);
    let qz = quat_axis([0.0, 0.0, 1.0], a[2]);
    quat_mul(qy, quat_mul(qx, qz))
}

/// `Quat_ToMtx`.
pub(crate) fn quat_to_mtx([x, y, z, w]: [f32; 4]) -> Mtx {
    let (xx, yy) = (x * x, y * y);
    [
        [
            1.0 - 2.0 * yy - 2.0 * (z * z),
            2.0 * x * y + 2.0 * w * z,
            2.0 * x * z - 2.0 * w * y,
            0.0,
        ],
        [
            2.0 * x * y - 2.0 * w * z,
            1.0 - 2.0 * xx - 2.0 * (z * z),
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

/// `Quat_ToEuler` (through `Mtx_ToEuler`).
pub(crate) fn quat_to_euler(q: [f32; 4]) -> [f32; 3] {
    let m = quat_to_mtx(q);
    let s = (-m[2][1]).clamp(-1.0, 1.0);
    let x = s.asin();
    if m[2][1] < 0.9999 {
        if m[2][1] > -0.9999 {
            [x, m[2][0].atan2(m[2][2]), m[0][1].atan2(m[1][1])]
        } else {
            [x, m[1][0].atan2(m[0][0]), 0.0]
        }
    } else {
        [x, -m[1][0].atan2(m[0][0]), 0.0]
    }
}

/// `Mtx_RotateX` on the identity (row vectors): rows (1, 0, 0), (0, c, s), (0, -s, c).
pub(crate) fn rot_x(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[1] = [0.0, c, s, 0.0];
    m[2] = [0.0, -s, c, 0.0];
    m
}

pub(crate) fn rot_y(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, 0.0, -s, 0.0];
    m[2] = [s, 0.0, c, 0.0];
    m
}

pub(crate) fn rot_z(a: f32) -> Mtx {
    let (s, c) = a.sin_cos();
    let mut m = IDENTITY;
    m[0] = [c, s, 0.0, 0.0];
    m[1] = [-s, c, 0.0, 0.0];
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_obb(center: [f32; 3]) -> Obb {
        let mut m = IDENTITY;
        m[3] = [center[0], center[1], center[2], 1.0];
        Obb::new([1.0; 3], &m)
    }

    #[test]
    fn sphere_touches_a_box_by_face_edge_and_corner_distance() {
        let obb = unit_obb([0.0; 3]);
        assert!(sphere_contacts_obb([1.5, 0.0, 0.0], 1.0, &obb));
        assert!(!sphere_contacts_obb([2.5, 0.0, 0.0], 1.0, &obb));
        // Off a corner by (0.6, 0.6, 0.6): 1.04 away.
        assert!(!sphere_contacts_obb([1.6, 1.6, 1.6], 1.0, &obb));
        assert!(sphere_contacts_obb([1.5, 1.5, 1.5], 1.0, &obb));
    }

    #[test]
    fn boxes_overlap_until_a_separating_axis_appears() {
        let a = unit_obb([0.0; 3]);
        assert!(obb_overlaps(&a, &unit_obb([1.9, 0.0, 0.0])));
        assert!(!obb_overlaps(&a, &unit_obb([2.1, 0.0, 0.0])));
        let mut m = rot_y(PI / 4.0);
        m[3] = [2.3, 0.0, 0.0, 1.0];
        // Turned by 45 degrees its corner reaches 1.41 towards the other box.
        assert!(obb_overlaps(&a, &Obb::new([1.0; 3], &m)));
    }

    #[test]
    fn euler_round_trip_through_the_quaternion() {
        let a = [0.3, -1.2, 0.5];
        let back = quat_to_euler(quat_from_euler(a));
        for i in 0..3 {
            assert!((a[i] - back[i]).abs() < 1e-4, "{a:?} {back:?}");
        }
    }

    #[test]
    fn event_masks_map_to_model_nodes_by_priority() {
        assert_eq!(mask_to_node(0x100 | 0x80), 0x14);
        assert_eq!(mask_to_node(0x8000), 0);
        assert_eq!(mask_to_node(0), 0);
    }
}
