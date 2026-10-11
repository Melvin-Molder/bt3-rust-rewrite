//! Battle motions (`BobjAnim`, include/battle/btl_obj_anim.h): one per motion id, BPE-packed in section `id + 1`
//! of the character's motion file (`0x598 + chara * 10`), sampled by `BtlObjAnim_SamplePose`.

use std::io;

/// Model node ids 0..70 can be animated (`BOBJ_NODE_MAX`).
pub const NODE_MAX: usize = 71;
/// Motions in a character's table (`BOBJ_ANIM_MAX`).
pub const MOTION_COUNT: usize = 0x19E;

#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    /// Rotation keys only: (frame, packed quaternion); the node keeps its rest translation.
    Rot(Vec<(u16, u64)>),
    /// (frame, translation, packed quaternion).
    Full(Vec<(u32, [f32; 3], u64)>),
}

/// `BobjAnimEvent`: an attribute word at a frame. Bit 0 opens a hit window (`arg` = attack nodes, `shape` its volume,
/// attribute bit 32 = sphere), bit 1 closes it; other bits are plain events whose `arg` is OR-ed per attribute.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionEvent {
    pub attr: u64,
    pub frame: u16,
    pub arg: u32,
    /// The volume record `shape` names: offset x, y, z in the node's space, then r0, r1, r2.
    pub shape: Option<[f32; 6]>,
}

pub const EV_HIT_BEGIN: u64 = 1;
pub const EV_HIT_END: u64 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    /// Last frame; the player's frame runs 0..length.
    pub length: u16,
    pub tracks: Vec<Option<Track>>,
    pub events: Vec<MotionEvent>,
}

impl Motion {
    /// Parses a decompressed motion.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let u16_at = |o: usize| -> io::Result<u16> {
            bytes
                .get(o..o + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| invalid_data("motion is truncated"))
        };
        let u32_at = |o: usize| -> io::Result<u32> {
            bytes
                .get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("motion is truncated"))
        };
        let u64_at = |o: usize| -> io::Result<u64> {
            Ok(u64::from(u32_at(o)?) | (u64::from(u32_at(o + 4)?) << 32))
        };
        let length = u16_at(2)?;
        let event_count = usize::from(bytes.get(1).copied().unwrap_or(0));
        let f32_at = |o: usize| u32_at(o).map(f32::from_bits);
        let events = (0..event_count)
            .map(|i| {
                let e = 0x94 + i * 0x10;
                let attr = u64_at(e)?;
                let shape = usize::from(u16_at(e + 0xA)?) * 4;
                // Only a hit window's opening event names a volume.
                let shape = if attr & EV_HIT_BEGIN != 0 {
                    Some([
                        f32_at(shape)?,
                        f32_at(shape + 4)?,
                        f32_at(shape + 8)?,
                        f32_at(shape + 0x10)?,
                        f32_at(shape + 0x14)?,
                        f32_at(shape + 0x18)?,
                    ])
                } else {
                    None
                };
                Ok(MotionEvent {
                    attr,
                    frame: u16_at(e + 8)?,
                    arg: u32_at(e + 0xC)?,
                    shape,
                })
            })
            .collect::<io::Result<_>>()?;
        let mut tracks = Vec::with_capacity(NODE_MAX);
        for node in 0..NODE_MAX {
            let offset = usize::from(u16_at(6 + node * 2)?) * 4;
            if offset == 0 {
                tracks.push(None);
                continue;
            }
            let flags = u16_at(offset)?;
            let count = usize::from(u16_at(offset + 2)?);
            let keys = offset + 4;
            tracks.push(Some(if flags & 1 != 0 {
                let times = keys + count * 8;
                Track::Rot(
                    (0..count)
                        .map(|i| Ok((u16_at(times + i * 2)?, u64_at(keys + i * 8)?)))
                        .collect::<io::Result<_>>()?,
                )
            } else {
                Track::Full(
                    (0..count)
                        .map(|i| {
                            let key = keys + i * 0x18;
                            let f = |o: usize| u32_at(key + o).map(f32::from_bits);
                            Ok((
                                u32_at(key + 0xC)?,
                                [f(0)?, f(4)?, f(8)?],
                                u64_at(key + 0x10)?,
                            ))
                        })
                        .collect::<io::Result<_>>()?,
                )
            }));
        }
        Ok(Self {
            length,
            tracks,
            events,
        })
    }

    /// `BtlAnim_ApplyTableFlags`: flag 0x40000000 zeroes the root's translation, else 0x80000000 makes it start
    /// at the origin (and flag 2 then zeroes its x).
    pub fn apply_table_flags(&mut self, flags: u32) {
        if flags & 0x4000_0000 != 0 {
            self.edit_root(|keys| keys.iter_mut().for_each(|k| k.1 = [0.0; 3]));
        } else if flags & 0x8000_0000 != 0 {
            self.edit_root(|keys| {
                let first = keys[0].1;
                for key in keys.iter_mut() {
                    key.1 = std::array::from_fn(|i| key.1[i] - first[i]);
                }
            });
            if flags & 2 != 0 {
                self.edit_root(|keys| keys.iter_mut().for_each(|k| k.1[0] = 0.0));
            }
        }
    }

    /// `BtlAnim_HideModelNodes`: models 0x78, 0x98 and 0x99 keep some nodes at the packed identity.
    pub fn hide_model_nodes(&mut self, model: i32) {
        let nodes: &[usize] = match model {
            0x98 => &[0x10, 0x11, 0x2E, 0x2F],
            0x99 => &[0x11],
            0x78 => &[0x11, 0x2E, 0x23, 0x15],
            _ => &[],
        };
        for &node in nodes {
            match self.tracks.get_mut(node).and_then(Option::as_mut) {
                Some(Track::Rot(keys)) => keys.iter_mut().for_each(|k| k.1 = PACKED_IDENTITY),
                Some(Track::Full(keys)) => keys.iter_mut().for_each(|k| k.2 = PACKED_IDENTITY),
                None => {}
            }
        }
    }

    /// The root node's translation keys, when its track has them (`BtlObjAnim_ZeroRootAxes` / `RebaseRoot`).
    fn edit_root(&mut self, edit: impl FnOnce(&mut Vec<(u32, [f32; 3], u64)>)) {
        if let Some(Some(Track::Full(keys))) = self.tracks.first_mut()
            && !keys.is_empty()
        {
            edit(keys);
        }
    }
}

/// `BtlObjAnim_ClearNodeRot`'s packed identity.
pub const PACKED_IDENTITY: u64 = 0x37FF_FF7F_FFF7_FFFF;

impl Track {
    fn len(&self) -> usize {
        match self {
            Track::Rot(keys) => keys.len(),
            Track::Full(keys) => keys.len(),
        }
    }

    fn frame(&self, i: usize) -> f32 {
        match self {
            Track::Rot(keys) => f32::from(keys[i].0),
            Track::Full(keys) => keys[i].0 as f32,
        }
    }

    fn key(&self, i: usize) -> (Option<[f32; 3]>, [f32; 4]) {
        match self {
            Track::Rot(keys) => (None, quat_unpack(keys[i].1)),
            Track::Full(keys) => (Some(keys[i].1), quat_unpack(keys[i].2)),
        }
    }

    /// `BtlObjAnim_SampleRot` / `BtlObjAnim_SamplePosRot`: the translation (`None` for a rotation-only track)
    /// and rotation at `frame`. The search starts at `cursor`, the key found last time. A frame outside every
    /// key gives translation (0, 0, 0) and the identity.
    pub fn sample(&self, cursor: &mut u16, frame: f32) -> (Option<[f32; 3]>, [f32; 4]) {
        let n = self.len();
        if n == 1 {
            return self.key(0);
        }
        let mut i = usize::from(*cursor);
        if i >= n - 1 {
            *cursor = 0;
            i = 0;
        }
        loop {
            for j in i + 1..n {
                let (fa, fb) = (self.frame(j - 1), self.frame(j));
                if fa == frame {
                    *cursor = (j - 1) as u16;
                    return self.key(j - 1);
                }
                if fa < frame && frame < fb {
                    let t = (frame - fa) / (fb - fa);
                    let (pa, qa) = self.key(j - 1);
                    let (pb, qb) = self.key(j);
                    *cursor = (j - 1) as u16;
                    let pos = pa
                        .zip(pb)
                        .map(|(a, b)| std::array::from_fn(|k| (b[k] - a[k]) * t + a[k]));
                    return (pos, quat_slerp(qa, qb, t));
                }
                if fb == frame {
                    *cursor = (j - 1) as u16;
                    return self.key(j);
                }
            }
            if *cursor == 0 {
                let pos = matches!(self, Track::Full(_)).then_some([0.0; 3]);
                return (pos, [0.0, 0.0, 0.0, 1.0]);
            }
            *cursor = 0;
            i = 0;
        }
    }
}

/// `Quat_Unpack`: "smallest three", three 20-bit fields and the index of the rebuilt component in bits 60..63.
pub fn quat_unpack(packed: u64) -> [f32; 4] {
    let largest = (packed >> 60) as usize & 3;
    let mut v = [0.0f32; 4];
    for (i, slot) in v.iter_mut().take(3).enumerate() {
        let n = ((packed >> (i * 20)) as u32 & 0xFFFFF) as f32;
        *slot = (n * (1.0 / 1_048_575.0) - 0.5) * 1.414_213_6;
    }
    let sum = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let w = (1.0 - sum).sqrt();
    for i in (largest + 1..4).rev() {
        v[i] = v[i - 1];
    }
    v[largest] = w;
    v
}

/// `Quat_Slerp`: from a (t = 0) to b (t = 1) along the shorter arc.
pub fn quat_slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = |p: [f32; 4], q: [f32; 4]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2] + p[3] * q[3];
    let mut b = b;
    let mut c = dot(a, b);
    if c < 0.0 {
        b = b.map(|x| -x);
        c = dot(a, b);
    }
    let angle = c.min(1.0).acos();
    let s = angle.sin();
    if s < 0.000_001 {
        return a;
    }
    let ka = (angle * (1.0 - t)).sin() / s;
    let kb = (angle * t).sin() / s;
    std::array::from_fn(|k| a[k] * ka + b[k] * kb)
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_identity_of_clear_node_rot_unpacks_to_the_identity() {
        // BtlObjAnim_ClearNodeRot writes this value as "the packed identity".
        let q = quat_unpack(0x37FF_FF7F_FFF7_FFFF);
        assert!(q[0].abs() < 1e-5 && q[1].abs() < 1e-5 && q[2].abs() < 1e-5);
        assert!((q[3] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sampling_between_keys_interpolates_and_outside_gives_rest() {
        let track = Track::Full(vec![
            (0, [0.0, 0.0, 0.0], 0x37FF_FF7F_FFF7_FFFF),
            (10, [10.0, 0.0, 0.0], 0x37FF_FF7F_FFF7_FFFF),
        ]);
        let mut cursor = 0;
        assert_eq!(track.sample(&mut cursor, 5.0).0, Some([5.0, 0.0, 0.0]));
        assert_eq!(track.sample(&mut cursor, 12.0).0, Some([0.0; 3]));
    }
}
