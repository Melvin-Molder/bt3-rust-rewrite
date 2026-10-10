//! Camera animations (`DemoCamAnim`, include/battle/btl_demo_cam.h): the stage's three intro cuts (stage file
//! sections 3..5) and a character's camera cuts (character file sections 0x29.., `mdl.camAnims`). Each channel
//! keys one word of the pose: position x, y, z, then rotations x, y, z.

use std::io;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamKey {
    /// 0x10: step (hold this value until the next key).
    pub flags: u32,
    pub time: f32,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamChannel {
    /// The pose word it drives.
    pub index: usize,
    pub keys: Vec<CamKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamAnim {
    pub length: f32,
    /// The pose before any channel is applied (8 words).
    pub defaults: [f32; 8],
    pub channels: Vec<CamChannel>,
}

impl CamAnim {
    /// Parses a camera animation; its offsets count from its own start (`DemoCam_FixupAnim`).
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let u32_at = |o: usize| -> io::Result<u32> {
            bytes
                .get(o..o + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid_data("camera animation is truncated"))
        };
        let f32_at = |o: usize| u32_at(o).map(f32::from_bits);
        let channels_at = u32_at(8)? as usize;
        let count = u32_at(0xC)? as usize;
        let defaults_at = u32_at(0x18)? as usize;
        let mut defaults = [0.0; 8];
        for (i, slot) in defaults.iter_mut().enumerate() {
            *slot = f32_at(defaults_at + i * 4)?;
        }
        let channels = (0..count)
            .map(|c| {
                let ch = channels_at + c * 0x10;
                let keys_at = u32_at(ch + 4)? as usize;
                let keys = (0..u32_at(ch + 8)? as usize)
                    .map(|k| {
                        let key = keys_at + k * 0x20;
                        Ok(CamKey {
                            flags: u32_at(key)?,
                            time: f32_at(key + 4)?,
                            value: f32_at(key + 8)?,
                        })
                    })
                    .collect::<io::Result<_>>()?;
                Ok(CamChannel {
                    index: u32_at(ch)? as usize,
                    keys,
                })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            length: f32_at(0x14)?,
            defaults,
            channels,
        })
    }

    /// `DemoCam_EvalAnim`: the defaults, then each channel at `time` (linear between keys, or held after a step
    /// key; before the first key its value, after the last the last value).
    pub fn eval(&self, time: f32) -> [f32; 8] {
        let mut out = self.defaults;
        for ch in &self.channels {
            let mut prev = None;
            let mut next = None;
            for key in &ch.keys {
                if key.time == time {
                    prev = Some(key);
                    break;
                }
                if key.time < time {
                    prev = Some(key);
                }
                if time < key.time {
                    next = Some(key);
                    break;
                }
            }
            let Some(slot) = out.get_mut(ch.index) else {
                continue;
            };
            match (prev, next) {
                (Some(p), Some(n)) if p.flags & 0x10 == 0 => {
                    *slot = (n.value - p.value) * ((time - p.time) / (n.time - p.time)) + p.value;
                }
                (Some(p), _) => *slot = p.value,
                (None, Some(n)) => *slot = n.value,
                (None, None) => {}
            }
        }
        out
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_interpolate_hold_and_clamp() {
        let anim = CamAnim {
            length: 10.0,
            defaults: [0.0; 8],
            channels: vec![
                CamChannel {
                    index: 0,
                    keys: vec![
                        CamKey {
                            flags: 0,
                            time: 0.0,
                            value: 0.0,
                        },
                        CamKey {
                            flags: 0,
                            time: 10.0,
                            value: 100.0,
                        },
                    ],
                },
                CamChannel {
                    index: 1,
                    keys: vec![
                        CamKey {
                            flags: 0x10,
                            time: 2.0,
                            value: 5.0,
                        },
                        CamKey {
                            flags: 0,
                            time: 8.0,
                            value: 9.0,
                        },
                    ],
                },
            ],
        };
        let pose = anim.eval(4.0);
        assert_eq!(pose[0], 40.0);
        assert_eq!(pose[1], 5.0);
        assert_eq!(anim.eval(1.0)[1], 5.0);
        assert_eq!(anim.eval(20.0)[0], 100.0);
    }
}
