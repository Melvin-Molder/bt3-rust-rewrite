//! The fighters' faces (`BtlObj_UpdateFace`, `BtlObj_SetSubState`, `BtlObj_SetEyeFrame`, src/battle/btl_obj_anim.c).
//! A face exists when the model has eye textures (character file entry 0xD) or a mouth (a mesh among entries
//! 4..11, a texture table among 0xE..0x11, or model flags 0xC00000: a jaw bone, node 0x31).
//!
//! The eyes blink: every 90..179 updates they shut for 3..5 (two libc `rand()` draws), unless object flag
//! 0x4000000 forces the frame `SetEyeFrame` asked for (9 hands them back) or flag 0x8000000 shuts them. The eye
//! frame is a texture of entry 0xD drawn in place of the model's face texture.
//!
//! The mouth runs in one of fifteen modes: 0 rest, 1 shut, 2 a lip track (which switches between 1 and 3 by its
//! records), 3 talking, 4 one talk pattern, 5..8 shapes 1..4, 9..12 shapes 5..8 held (left only through 0 or
//! another of them; 10 also opens the jaw 21 degrees), 13 three talk patterns. A talk pattern is one of two
//! records of {frame, shape} picked by `rand() & 1`, 2 frames an update; a jaw model also picks one of four jaw
//! tracks (`rand() % 4`, model flag 0x400000 the second set) and turns node 0x31 by it instead of changing shape.
//! A shape whose mesh exists replaces the mouth part (node 0x30); shapes 5..8 also replace the face texture.

use std::io;
use std::sync::Arc;

use bt3_data::elf::Elf;
use bt3_data::face::LipKey;

use crate::rng::LibcRandom;

/// `gBtlObjTalkPattern7` / `gBtlObjTalkPattern5`: {frame, shape} records.
const TALK_PATTERN_7: u32 = 0x2C_6C20;
const TALK_PATTERN_5: u32 = 0x2C_6C40;
/// `gBtlObjJawTrack0..3`, then `gBtlObjJawTrackAlt0..3`: four {x, y, z, frame} records each.
const JAW_TRACKS: u32 = 0x2C_6C58;
/// The records the jaw loop can reach: it runs to the talk pattern's length (7) from the last track's start.
const JAW_RECORDS: usize = 7 * 4 + 7;

pub(crate) const FACE_EYES: u32 = 1;
pub(crate) const FACE_MOUTH: u32 = 2;
/// Object flags: the eye frame is the one asked for; the eyes are shut.
pub(crate) const FLAG_EYE_FORCED: u32 = 0x400_0000;
pub(crate) const FLAG_EYE_SHUT: u32 = 0x800_0000;

/// What the executable gives every face.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FaceTables {
    pub talk7: Vec<[i16; 2]>,
    pub talk5: Vec<[i16; 2]>,
    /// Euler angles and frame.
    pub jaw: Vec<([f32; 3], u32)>,
}

impl FaceTables {
    pub(crate) fn read(elf: &Elf) -> io::Result<Self> {
        let pairs = |at: u32, n: usize| -> io::Result<Vec<[i16; 2]>> {
            let b = elf.read(at, n * 4)?;
            Ok(b.chunks_exact(4)
                .map(|p| {
                    [
                        i16::from_le_bytes([p[0], p[1]]),
                        i16::from_le_bytes([p[2], p[3]]),
                    ]
                })
                .collect())
        };
        let b = elf.read(JAW_TRACKS, JAW_RECORDS * 16)?;
        let jaw = b
            .chunks_exact(16)
            .map(|r| {
                let f = |o: usize| f32::from_le_bytes(r[o..o + 4].try_into().unwrap());
                (
                    [f(0), f(4), f(8)],
                    u32::from_le_bytes(r[12..16].try_into().unwrap()),
                )
            })
            .collect();
        Ok(Self {
            talk7: pairs(TALK_PATTERN_7, 7)?,
            talk5: pairs(TALK_PATTERN_5, 5)?,
            jaw,
        })
    }
}

/// What a model gives its face (`BObjMdl`'s face tables).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FaceModel {
    /// Entry 0xD's texture count; `None` when the entry is empty.
    pub eye_count: Option<u32>,
    /// Per mouth shape 1..8: whether its mesh exists (`mouthA`) and, for 5..8, its texture table (`mouthB`).
    pub mouth_mesh: [bool; 8],
    pub mouth_tex: [bool; 8],
    /// The model header's flags (0xC00000: a jaw; 0x400000: the second set of jaw tracks).
    pub model_flags: u32,
    pub lips: Vec<Option<Vec<LipKey>>>,
    pub tables: FaceTables,
}

impl FaceModel {
    /// `BtlObjMdl_HasJaw`.
    pub(crate) fn has_jaw(&self) -> bool {
        self.model_flags & 0xC0_0000 != 0
    }

    /// `BtlObjMdl_HasMouth`.
    fn has_mouth(&self) -> bool {
        self.mouth_mesh.iter().chain(&self.mouth_tex).any(|&b| b) || self.has_jaw()
    }

    fn lip(&self, n: i32) -> Option<&[LipKey]> {
        let n = usize::try_from(n).ok()?;
        self.lips.get(n)?.as_deref()
    }
}

/// The face texture drawn in place of the model's own: an eye frame (0-based) or a mouth shape's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaceTex {
    Eye(usize),
    Mouth(usize),
}

/// Where the lip track being played comes from: the model's track by line (mode 2), or one handed in (mode 14,
/// the story script's lip file).
#[derive(Clone, Debug)]
enum LipTrack {
    Model(usize),
    Own(Arc<[LipKey]>),
}

/// `BObjFace`.
#[derive(Clone, Debug)]
pub(crate) struct Face {
    model: Arc<FaceModel>,
    pub flags: u32,
    blink_wait: i32,
    blink_time: i32,
    /// 1-based eye texture, 0 for the model's own.
    pub eye: i32,
    pub eye_forced: u32,
    pub mode: i32,
    /// 1-based mouth shape, 0 shut.
    pub shape: i32,
    /// The first record of the jaw track in use.
    jaw_keys: Option<usize>,
    step: f32,
    talk_end: f32,
    talk_time: f32,
    /// 7 or 5 for the talk pattern in use, 0 for none.
    talk_count: usize,
    /// The lip track being played (`lipArg`).
    lip: Option<LipTrack>,
    lip_index: u16,
    lip_count: u16,
    lip_time: f32,
    lip_end: f32,
    /// Euler angles of node 0x31.
    pub jaw: [f32; 3],
    talk_loops: i32,
}

impl Face {
    /// `BtlObj_CreateFace`: `None` when the model has neither eyes nor a mouth.
    /// `BtlObj_CopyLipTables`: the lip tracks of another face's model.
    pub(crate) fn copy_lips(&mut self, from: &Face) {
        let mut model = (*self.model).clone();
        model.lips = from.model.lips.clone();
        self.model = Arc::new(model);
    }

    pub(crate) fn new(model: Arc<FaceModel>) -> Option<Self> {
        let mut flags = 0;
        if model.eye_count.is_some() {
            flags |= FACE_EYES;
        }
        if model.has_mouth() {
            flags |= FACE_MOUTH;
        }
        if flags == 0 {
            return None;
        }
        let mut face = Self {
            model,
            flags,
            blink_wait: 0,
            blink_time: 0,
            eye: 0,
            eye_forced: 0,
            mode: 0,
            shape: 0,
            jaw_keys: None,
            step: 0.0,
            talk_end: 0.0,
            talk_time: 0.0,
            talk_count: 0,
            lip: None,
            lip_index: 0,
            lip_count: 0,
            lip_time: 0.0,
            lip_end: 0.0,
            jaw: [0.0; 3],
            talk_loops: 0,
        };
        if flags & FACE_MOUTH != 0 {
            face.reset(false);
        }
        Some(face)
    }

    /// `BtlObjFace_Reset`: the talk stops; without `keep` the mode, lip track and jaw go too.
    fn reset(&mut self, keep: bool) {
        self.talk_end = 0.0;
        self.shape = 0;
        self.talk_time = 0.0;
        self.talk_count = 0;
        self.talk_loops = 0;
        if !keep {
            self.mode = 0;
            self.lip = None;
            self.lip_time = 0.0;
            self.lip_end = 0.0;
            self.lip_count = 0;
            self.lip_index = 0;
            self.jaw = [0.0; 3];
        }
        self.step = 2.0;
    }

    fn talk(&self) -> &[[i16; 2]] {
        match self.talk_count {
            7 => &self.model.tables.talk7,
            5 => &self.model.tables.talk5,
            _ => &[],
        }
    }

    /// A talk pattern by `rand() & 1`.
    fn pick_talk(&mut self, libc: &mut LibcRandom) {
        self.talk_count = if libc.rand() & 1 != 0 { 7 } else { 5 };
    }

    /// `BtlObjFace_PickJawKeys`.
    fn pick_jaw(&mut self, libc: &mut LibcRandom) {
        if !self.model.has_jaw() {
            return;
        }
        let set = if self.model.model_flags & 0x40_0000 != 0 {
            4
        } else {
            0
        };
        self.jaw_keys = Some((set + (libc.rand() % 4) as usize) * 4);
    }

    /// `BtlObjFace_CanStart`: modes 9..12 only yield to 0 or 9..12.
    fn can_start(&self, mode: i32) -> bool {
        !(9..=12).contains(&self.mode) || (9..=12).contains(&mode) || mode == 0
    }

    /// `BtlObj_SetSubState`: mode 2 plays lip track `arg` (none when the model lacks it).
    pub(crate) fn set_sub_state(&mut self, mode: i32, arg: i32, libc: &mut LibcRandom) {
        if self.flags & FACE_MOUTH == 0 {
            return;
        }
        if arg >= 0 && mode == 2 && self.model.lip(arg).is_none() {
            return;
        }
        if !self.can_start(mode) {
            return;
        }
        self.mode = mode;
        match mode {
            0 => self.reset(false),
            1 => self.reset(true),
            3 | 4 | 13 => {
                self.pick_talk(libc);
                self.talk_end = f32::from(self.talk().last().map_or(0, |r| r[0]));
                self.talk_time = 0.0;
                self.talk_loops = 0;
                self.pick_jaw(libc);
            }
            2 => {
                let Some(keys) = self.model.lip(arg) else {
                    return;
                };
                self.lip_index = 0;
                self.lip_count = keys.len() as u16;
                self.lip_end = f32::from(keys.last().map_or(0, |k| k.frame));
                self.lip = Some(LipTrack::Model(arg as usize));
                self.lip_time = 0.0;
                self.pick_jaw(libc);
            }
            5..=8 => self.shape = mode - 4,
            9..=12 => {
                self.reset(false);
                self.mode = mode;
                self.shape = mode - 4;
                if mode == 10 {
                    self.jaw = [0.366_519_1, 0.0, 0.0];
                }
            }
            _ => {}
        }
    }

    /// `BtlObj_SetSubState(obj, BOBJ_MOUTH_LIP_PTR, keys)`: mode 14 plays the lip track handed in.
    pub(crate) fn play_lip(&mut self, keys: Arc<[LipKey]>, libc: &mut LibcRandom) {
        if self.flags & FACE_MOUTH == 0 || !self.can_start(14) {
            return;
        }
        self.mode = 14;
        self.lip_index = 0;
        self.lip_count = keys.len() as u16;
        self.lip_end = f32::from(keys.last().map_or(0, |k| k.frame));
        self.lip = Some(LipTrack::Own(keys));
        self.lip_time = 0.0;
        self.pick_jaw(libc);
    }

    /// `BtlObj_SetSubState` with a mode that draws no random number (0, 1, 5..12).
    pub(crate) fn set_mode(&mut self, mode: i32) {
        debug_assert!(!matches!(mode, 2..=4 | 13 | 14));
        self.set_sub_state(mode, 0, &mut LibcRandom::new(0));
    }

    /// `BtlObj_SetEyeFrame`: forces eye frame `frame` (when the table has it); 9 hands the eyes back to the blink.
    pub(crate) fn set_eye_frame(&mut self, obj_flags: &mut u32, frame: u32) {
        if self.flags & FACE_EYES == 0 {
            return;
        }
        if frame != 9 {
            *obj_flags |= FLAG_EYE_FORCED;
            if frame <= self.model.eye_count.unwrap_or(0) {
                self.eye_forced = frame;
            }
        } else {
            *obj_flags &= !FLAG_EYE_FORCED;
        }
    }

    /// `BtlObj_UpdateFace`: the blink, the lip track, the mouth.
    pub(crate) fn update(&mut self, obj_flags: u32, libc: &mut LibcRandom) {
        if self.flags & FACE_EYES != 0 {
            self.step_blink(obj_flags, libc);
        }
        if self.flags & FACE_MOUTH != 0 {
            self.step_lip(libc);
            self.step_mouth(libc);
        }
    }

    /// `BtlObjFace_StepBlink`.
    fn step_blink(&mut self, obj_flags: u32, libc: &mut LibcRandom) {
        if obj_flags & FLAG_EYE_FORCED != 0 {
            let n = self.eye_forced.wrapping_sub(1);
            self.eye = if n < self.model.eye_count.unwrap_or(0) && (n as i32) >= 0 {
                self.eye_forced as i32
            } else {
                0
            };
        } else if obj_flags & FLAG_EYE_SHUT != 0 {
            self.eye = 1;
        } else {
            let wait = self.blink_wait;
            self.blink_wait -= 1;
            if wait < 0 {
                self.blink_time = libc.rand() % 3 + 3;
                self.blink_wait = libc.rand() % 90 + 90;
            }
            if self.blink_time != 0 {
                let time = self.blink_time;
                self.blink_time -= 1;
                if time < 0 {
                    self.blink_time = 0;
                    self.eye = 0;
                } else {
                    self.eye = 1;
                }
            } else {
                self.eye = 0;
            }
        }
    }

    /// `BObjFace_StepLipData`: the record the track's time falls in opens (talk) or shuts the mouth; past the
    /// end (or outside every record) the mouth rests.
    fn step_lip(&mut self, libc: &mut LibcRandom) {
        let Some(track) = self.lip.clone() else {
            return;
        };
        let model = Arc::clone(&self.model);
        let keys = match &track {
            LipTrack::Model(i) => model.lips[*i].as_deref().unwrap_or(&[]),
            LipTrack::Own(keys) => keys,
        };
        let time = self.lip_time;
        if 0.0 <= time {
            let mut found = None;
            let last = i32::from(self.lip_count) - 1;
            let mut i = i32::from(self.lip_index);
            while i < last {
                let (a, b) = (keys[i as usize], keys[i as usize + 1]);
                if f32::from(a.frame) <= time && time < f32::from(b.frame) {
                    found = Some(i as usize);
                    self.lip_index = i as u16;
                    break;
                }
                i += 1;
            }
            let Some(found) = found.filter(|_| !(self.lip_end < time)) else {
                self.set_sub_state(0, 0, libc);
                return;
            };
            if keys[found].code == 0 {
                self.set_sub_state(1, 0, libc);
            } else if self.mode != 3 {
                self.set_sub_state(3, 0, libc);
            }
        }
        self.lip_time += self.step;
    }

    /// `BtlObjFace_Step`.
    fn step_mouth(&mut self, libc: &mut LibcRandom) {
        match self.mode {
            0 => {
                self.reset(false);
                return;
            }
            3 => {
                self.talk_time += self.step;
                if self.talk_end <= self.talk_time {
                    self.pick_talk(libc);
                    self.talk_time = 0.0;
                    self.shape = i32::from(self.talk()[0][1]);
                }
            }
            4 => {
                self.talk_time += self.step;
                if self.talk_end <= self.talk_time {
                    self.reset(false);
                }
            }
            13 => {
                self.talk_time += self.step;
                if self.talk_end <= self.talk_time {
                    let loops = self.talk_loops;
                    self.talk_loops += 1;
                    if loops < 3 {
                        self.pick_talk(libc);
                        self.talk_time = 0.0;
                        self.shape = i32::from(self.talk()[0][1]);
                    } else if self.talk_time >= self.talk_end {
                        self.reset(false);
                    }
                }
            }
            10 => {
                self.jaw = [0.366_519_1, 0.0, 0.0];
                return;
            }
            1 | 2 | 14 => {}
            _ => return,
        }
        let time = self.talk_time;
        if self.model.has_jaw() {
            if let Some(start) = self.jaw_keys {
                let keys = &self.model.tables.jaw;
                for i in 1..self.talk_count {
                    let (Some(&(a, fa)), Some(&(b, fb))) =
                        (keys.get(start + i - 1), keys.get(start + i))
                    else {
                        break;
                    };
                    let (fa, fb) = (fa as f32, fb as f32);
                    if fa == time {
                        self.jaw = a;
                        break;
                    }
                    if fa < time && time < fb {
                        let t = (time - fa) / (fb - fa);
                        self.jaw = std::array::from_fn(|k| (b[k] - a[k]) * t + a[k]);
                        break;
                    }
                    if fb == time {
                        self.jaw = b;
                        break;
                    }
                }
            }
            self.shape = 0;
        } else {
            let talk = self.talk();
            for i in 1..self.talk_count {
                let (a, b) = (talk[i - 1], talk[i]);
                if f32::from(a[0]) <= time && time <= f32::from(b[0]) {
                    self.shape = i32::from(a[1]);
                    return;
                }
            }
        }
    }

    /// `BtlObj_GetSubState`: the shape in use minus one, or -1 (shut, or the model lacks that mesh).
    pub(crate) fn sub_state(&self) -> i32 {
        let i = self.shape - 1;
        if i >= 0
            && self
                .model
                .mouth_mesh
                .get(i as usize)
                .copied()
                .unwrap_or(false)
        {
            i
        } else {
            -1
        }
    }

    /// `BtlObj_GetFaceTexture`: shapes 5..8 their table's texture, else the eye frame.
    pub(crate) fn texture(&self) -> Option<FaceTex> {
        let shape = self.sub_state();
        if shape >= 4 {
            let i = shape as usize;
            self.model.mouth_tex[i].then_some(FaceTex::Mouth(i))
        } else if self.eye != 0 {
            Some(FaceTex::Eye(self.eye as usize - 1))
        } else {
            None
        }
    }

    /// `BtlObj_IsJawActive`.
    pub(crate) fn jaw_active(&self) -> bool {
        self.mode != 0 && self.model.has_jaw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> FaceTables {
        FaceTables {
            talk7: vec![[0, 0], [3, 2], [6, 3], [9, 4], [15, 3], [18, 2], [21, 0]],
            talk5: vec![[0, 0], [5, 2], [9, 3], [17, 2], [21, 0]],
            jaw: (0..JAW_RECORDS)
                .map(|i| ([i as f32, 0.0, 0.0], (i % 4) as u32 * 10))
                .collect(),
        }
    }

    fn face(model_flags: u32) -> Face {
        let mut lips = vec![None; 100];
        lips[3] = Some(vec![
            LipKey { frame: 0, code: 1 },
            LipKey { frame: 6, code: 0 },
            LipKey { frame: 10, code: 0 },
        ]);
        Face::new(Arc::new(FaceModel {
            eye_count: Some(3),
            mouth_mesh: [true; 8],
            mouth_tex: [false, false, false, false, true, true, true, true],
            model_flags,
            lips,
            tables: tables(),
        }))
        .unwrap()
    }

    #[test]
    fn blink_draws_two_numbers_and_shuts_the_eyes_for_their_count() {
        let mut f = face(0);
        let mut libc = LibcRandom::new(7);
        let mut check = LibcRandom::new(7);
        // The first update counts the wait down from 0 to -1; the second draws.
        f.update(0, &mut libc);
        assert_eq!(f.eye, 0);
        f.update(0, &mut libc);
        let shut = check.rand() % 3 + 3;
        let wait = check.rand() % 90 + 90;
        assert_eq!((f.blink_wait, f.eye), (wait, 1));
        let mut frames = 1;
        while f.eye == 1 {
            f.update(0, &mut libc);
            frames += 1;
        }
        assert_eq!(frames, shut + 1);
        // Forced frames within the table, the model's own beyond it.
        let mut flags = 0;
        f.set_eye_frame(&mut flags, 2);
        f.update(flags, &mut libc);
        assert_eq!((flags, f.eye), (FLAG_EYE_FORCED, 2));
        f.set_eye_frame(&mut flags, 9);
        assert_eq!(flags, 0);
    }

    #[test]
    fn talk_once_walks_the_pattern_and_rests() {
        let mut f = face(0);
        let mut libc = LibcRandom::new(1);
        let seven = LibcRandom::new(1).rand() & 1 != 0;
        f.set_sub_state(4, 0, &mut libc);
        let mut shapes = Vec::new();
        while f.mode == 4 {
            f.update(FLAG_EYE_FORCED, &mut libc);
            shapes.push(f.shape);
        }
        let want: &[i32] = if seven {
            &[0, 2, 2, 3, 4, 4, 4, 3, 3, 2, 0]
        } else {
            &[0, 0, 2, 2, 3, 3, 3, 3, 2, 2, 0]
        };
        assert_eq!(shapes, want);
        assert_eq!(f.sub_state(), -1);
    }

    #[test]
    fn held_shapes_yield_only_to_rest_and_replace_the_face_texture() {
        let mut f = face(0);
        let mut libc = LibcRandom::new(1);
        f.set_mode(9);
        f.set_mode(5);
        assert_eq!(
            (f.mode, f.sub_state(), f.texture()),
            (9, 4, Some(FaceTex::Mouth(4)))
        );
        f.set_sub_state(3, 0, &mut libc);
        assert_eq!(f.mode, 9);
        f.set_mode(0);
        f.eye = 2;
        assert_eq!((f.mode, f.texture()), (0, Some(FaceTex::Eye(1))));
    }

    #[test]
    fn lip_track_talks_then_shuts_then_rests() {
        let mut f = face(0);
        let mut libc = LibcRandom::new(1);
        f.set_sub_state(2, 50, &mut libc);
        assert_eq!(f.mode, 0, "no such track");
        f.set_sub_state(2, 3, &mut libc);
        let mut modes = Vec::new();
        for _ in 0..8 {
            f.update(FLAG_EYE_FORCED, &mut libc);
            modes.push(f.mode);
        }
        // Frames 0, 2, 4 talk; 6, 8 shut; 10 is the last record's frame (no record starts there): rest.
        assert_eq!(modes, [3, 3, 3, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn jaw_models_turn_the_jaw_along_the_picked_track() {
        let mut f = face(0x80_0000);
        let mut libc = LibcRandom::new(3);
        let mut check = LibcRandom::new(3);
        let count = if check.rand() & 1 != 0 { 7 } else { 5 };
        let start = (check.rand() % 4) as usize * 4;
        f.set_sub_state(3, 0, &mut libc);
        assert!(f.jaw_active());
        assert_eq!((f.talk_count, f.jaw_keys), (count, Some(start)));
        // Time 2 lies between the track's frames 0 and 10: a fifth of the way from record 0 to record 1.
        f.update(FLAG_EYE_FORCED, &mut libc);
        assert_eq!(f.shape, 0);
        let want = start as f32 + 0.2;
        assert!((f.jaw[0] - want).abs() < 1e-5, "{:?}", f.jaw);
        f.set_mode(10);
        f.update(FLAG_EYE_FORCED, &mut libc);
        assert_eq!(f.jaw, [0.366_519_1, 0.0, 0.0]);
    }
}
